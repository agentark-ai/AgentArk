#![allow(dead_code)]

use super::*;
use crate::core::automation::AutomationCritique;
use chrono::TimeZone;

fn action_is_read_only(action: &crate::actions::ActionDef) -> bool {
    matches!(
        action.action_metadata().side_effect_level,
        ActionSideEffectLevel::None
    )
}

fn required_argument_present(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Null) | None => false,
        Some(serde_json::Value::String(s)) => !s.trim().is_empty(),
        Some(serde_json::Value::Array(items)) => !items.is_empty(),
        Some(serde_json::Value::Object(map)) => !map.is_empty(),
        Some(_) => true,
    }
}

fn missing_required_fields(
    action: &crate::actions::ActionDef,
    payload: &serde_json::Map<String, serde_json::Value>,
) -> Vec<String> {
    let required = action
        .input_schema
        .get("required")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    required
        .into_iter()
        .filter(|field| !required_argument_present(payload.get(field.as_str())))
        .collect()
}

fn action_is_scheduler_default_candidate(action: &crate::actions::ActionDef) -> bool {
    let metadata = action.action_metadata();
    let read_only_source = matches!(metadata.side_effect_level, ActionSideEffectLevel::None)
        && matches!(
            metadata.role,
            ActionRole::DataSource | ActionRole::Inspection | ActionRole::Trigger
        );
    let internal_notification = matches!(metadata.side_effect_level, ActionSideEffectLevel::Notify)
        && matches!(metadata.role, ActionRole::Delivery)
        && matches!(metadata.integration_class, ActionIntegrationClass::Internal);
    read_only_source || internal_notification
}

fn watcher_cadence_label(interval_secs: u64) -> String {
    if interval_secs == 0 {
        return "as often as the watcher scheduler allows".to_string();
    }
    if interval_secs == 1 {
        return "every second".to_string();
    }
    if interval_secs < 60 {
        return format!("every {} seconds", interval_secs);
    }
    if interval_secs == 60 {
        return "every minute".to_string();
    }
    if interval_secs < 3600 {
        let minutes = interval_secs / 60;
        let seconds = interval_secs % 60;
        if seconds == 0 {
            return format!("every {} minutes", minutes);
        }
        return format!("every {} minutes {} seconds", minutes, seconds);
    }
    if interval_secs == 3600 {
        return "hourly".to_string();
    }
    if interval_secs == 12 * 3600 {
        return "twice a day".to_string();
    }
    if interval_secs == 24 * 3600 {
        return "daily".to_string();
    }
    if interval_secs < 24 * 3600 {
        let hours = interval_secs / 3600;
        let minutes = (interval_secs % 3600) / 60;
        if minutes == 0 {
            return format!("every {} hours", hours);
        }
        return format!("every {} hours {} minutes", hours, minutes);
    }

    let days = interval_secs / (24 * 3600);
    let hours = (interval_secs % (24 * 3600)) / 3600;
    if hours == 0 {
        format!("every {} days", days)
    } else {
        format!("every {} days {} hours", days, hours)
    }
}

fn watcher_delivery_sentence_fragment(notify_channel: &str) -> String {
    let normalized = notify_channel.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized == AUTOMATION_IN_APP_NOTIFICATION_CHANNEL {
        "in app only".to_string()
    } else if normalized == "preferred" {
        "through the preferred connected channel, falling back to in-app notification".to_string()
    } else {
        watcher_delivery_label(&normalized)
    }
}

fn watcher_repeat_on_match_from_arguments(
    arguments: &serde_json::Value,
    existing_watcher: Option<&super::watcher::Watcher>,
    until_stopped: bool,
) -> bool {
    if let Some(value) = arguments
        .get("repeat_on_match")
        .and_then(|value| value.as_bool())
    {
        return value;
    }

    if let Some(watcher) = existing_watcher {
        return watcher.repeat_on_match;
    }

    until_stopped
}

fn schedule_task_batch_item_arguments(
    arguments: &serde_json::Value,
) -> Option<Result<Vec<serde_json::Value>>> {
    let items = arguments.get("items")?;
    let Some(items) = items.as_array().filter(|items| !items.is_empty()) else {
        return Some(Err(anyhow::anyhow!(
            "schedule_task.items must be a non-empty array"
        )));
    };
    let inherited_keys = [
        "task",
        "report_to",
        "action",
        "action_arguments",
        "script",
        "script_language",
        "context_from",
        "workdir",
        "network_access",
        "scheduled_for",
        "local_date",
        "local_time",
        "timezone",
        "timezone_offset_minutes",
        "date_policy",
        "allow_duplicate",
        "validation",
        "max_attempts",
        "stall_timeout_secs",
        "retry_backoff_secs",
        "automation_policy",
    ];
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(item_obj) = item.as_object() else {
            return Some(Err(anyhow::anyhow!(
                "schedule_task.items[{}] must be an object",
                index
            )));
        };
        let mut merged = serde_json::Map::new();
        for key in inherited_keys {
            if let Some(value) = arguments.get(key) {
                merged.insert(key.to_string(), value.clone());
            }
        }
        for (key, value) in item_obj {
            merged.insert(key.clone(), value.clone());
        }
        merged.remove("items");
        let merged = serde_json::Value::Object(merged);
        let has_task_ref = schedule_task_has_task_source(&merged);
        let has_schedule = schedule_task_has_schedule_source(&merged);
        if !has_task_ref || !has_schedule {
            return Some(Err(anyhow::anyhow!(
                "schedule_task.items[{}] must identify a task and a schedule",
                index
            )));
        }
        out.push(merged);
    }
    Some(Ok(out))
}

fn schedule_text_field<'a>(arguments: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .or_else(|| arguments.get("schedule").and_then(|value| value.get(key)))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn schedule_i32_field(arguments: &serde_json::Value, key: &str) -> Option<i32> {
    arguments
        .get(key)
        .or_else(|| arguments.get("schedule").and_then(|value| value.get(key)))
        .and_then(|value| {
            value
                .as_i64()
                .and_then(|number| i32::try_from(number).ok())
                .or_else(|| value.as_str()?.trim().parse::<i32>().ok())
        })
}

fn schedule_task_has_task_source(arguments: &serde_json::Value) -> bool {
    arguments
        .get("task")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || arguments
            .get("task_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        || arguments
            .get("action_arguments")
            .and_then(|value| value.get("message"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
}

fn schedule_task_has_schedule_source(arguments: &serde_json::Value) -> bool {
    schedule_text_field(arguments, "cron").is_some()
        || schedule_text_field(arguments, "at").is_some()
        || schedule_text_field(arguments, "scheduled_for").is_some()
        || schedule_text_field(arguments, "local_time").is_some()
}

fn watch_batch_item_arguments(
    arguments: &serde_json::Value,
) -> Option<Result<Vec<serde_json::Value>>> {
    let items = arguments.get("items")?;
    let Some(items) = items.as_array().filter(|items| !items.is_empty()) else {
        return Some(Err(anyhow::anyhow!(
            "watch.items must be a non-empty array"
        )));
    };
    let inherited_keys = [
        "description",
        "poll_action",
        "poll_arguments",
        "script",
        "script_language",
        "context_from",
        "workdir",
        "network_access",
        "condition",
        "on_trigger",
        "interval_secs",
        "timeout_secs",
        "timeout_hours",
        "timeout_days",
        "until_stopped",
        "notify_channel",
        "repeat_on_match",
        "allow_duplicate",
        "validation",
        "max_attempts",
        "stall_timeout_secs",
        "retry_backoff_secs",
        "automation_policy",
    ];
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(item_obj) = item.as_object() else {
            return Some(Err(anyhow::anyhow!(
                "watch.items[{}] must be an object",
                index
            )));
        };
        let mut merged = serde_json::Map::new();
        for key in inherited_keys {
            if let Some(value) = arguments.get(key) {
                merged.insert(key.to_string(), value.clone());
            }
        }
        for (key, value) in item_obj {
            merged.insert(key.clone(), value.clone());
        }
        merged.remove("items");
        let merged = serde_json::Value::Object(merged);
        let updates_existing = merged
            .get("watcher_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        let has_poll_action = merged
            .get("poll_action")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        let has_script = merged
            .get("script")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        let can_create = merged
            .get("description")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            && (has_poll_action || has_script)
            && merged.get("condition").is_some()
            && merged
                .get("on_trigger")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());
        if !updates_existing && !can_create {
            return Some(Err(anyhow::anyhow!(
                "watch.items[{}] must either identify a watcher_id or include description, poll_action/script, condition, and on_trigger",
                index
            )));
        }
        out.push(merged);
    }
    Some(Ok(out))
}

fn schedule_task_completion_data(result: &str) -> Option<serde_json::Value> {
    let payload = result
        .trim_start()
        .strip_prefix(crate::runtime::TOOL_COMPLETION_MARKER)?
        .lines()
        .next()
        .unwrap_or_default()
        .trim();
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()?
        .get("data")
        .cloned()
}

#[derive(Clone, Copy)]
struct ScheduleTaskScheduleContext<'a> {
    now_utc: chrono::DateTime<chrono::Utc>,
    existing_task_target: Option<&'a super::task::Task>,
    default_timezone: Option<chrono_tz::Tz>,
}

impl Default for ScheduleTaskScheduleContext<'_> {
    fn default() -> Self {
        Self {
            now_utc: chrono::Utc::now(),
            existing_task_target: None,
            default_timezone: None,
        }
    }
}

enum ScheduleTaskTimezone {
    Named(chrono_tz::Tz),
    Fixed(chrono::FixedOffset),
}

impl ScheduleTaskTimezone {
    fn from_arguments(
        arguments: &serde_json::Value,
        default_timezone: Option<chrono_tz::Tz>,
    ) -> Result<Self, String> {
        if let Some(offset_minutes) = schedule_i32_field(arguments, "timezone_offset_minutes") {
            let Some(offset) = chrono::FixedOffset::east_opt(offset_minutes.saturating_mul(60))
            else {
                return Err(format!(
                    "Invalid schedule timezone_offset_minutes `{}`.",
                    offset_minutes
                ));
            };
            return Ok(Self::Fixed(offset));
        }

        if let Some(timezone) = schedule_text_field(arguments, "timezone") {
            let parsed = timezone.parse::<chrono_tz::Tz>().map_err(|_| {
                format!(
                    "Invalid schedule timezone `{}`. Use an IANA timezone such as Asia/Kolkata or provide timezone_offset_minutes.",
                    timezone
                )
            })?;
            return Ok(Self::Named(parsed));
        }

        Ok(Self::Named(default_timezone.unwrap_or(chrono_tz::UTC)))
    }

    fn local_date_for_utc(&self, value: chrono::DateTime<chrono::Utc>) -> chrono::NaiveDate {
        match self {
            Self::Named(timezone) => value.with_timezone(timezone).date_naive(),
            Self::Fixed(offset) => value.with_timezone(offset).date_naive(),
        }
    }

    fn resolve_local_datetime(
        &self,
        local: chrono::NaiveDateTime,
    ) -> Result<chrono::DateTime<chrono::Utc>, String> {
        match self {
            Self::Named(timezone) => match timezone.from_local_datetime(&local) {
                chrono::LocalResult::Single(value) => Ok(value.with_timezone(&chrono::Utc)),
                chrono::LocalResult::Ambiguous(earliest, _) => {
                    Ok(earliest.with_timezone(&chrono::Utc))
                }
                chrono::LocalResult::None => Err(format!(
                    "Local schedule time `{}` does not exist in the selected timezone.",
                    local
                )),
            },
            Self::Fixed(offset) => match offset.from_local_datetime(&local) {
                chrono::LocalResult::Single(value) => Ok(value.with_timezone(&chrono::Utc)),
                chrono::LocalResult::Ambiguous(earliest, _) => {
                    Ok(earliest.with_timezone(&chrono::Utc))
                }
                chrono::LocalResult::None => Err(format!(
                    "Local schedule time `{}` does not exist in the selected timezone.",
                    local
                )),
            },
        }
    }
}

fn parse_schedule_local_time(value: &str) -> Result<chrono::NaiveTime, String> {
    let normalized = value.trim().to_ascii_uppercase();
    for format in [
        "%H:%M:%S",
        "%H:%M",
        "%I:%M:%S %p",
        "%I:%M %p",
        "%I:%M%p",
        "%I %p",
    ] {
        if let Ok(time) = chrono::NaiveTime::parse_from_str(&normalized, format) {
            return Ok(time);
        }
    }
    Err(format!(
        "Invalid schedule local_time `{}`. Use HH:MM, HH:MM:SS, or a standard AM/PM time.",
        value
    ))
}

fn parse_schedule_local_date(value: &str) -> Result<chrono::NaiveDate, String> {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|error| {
        format!(
            "Invalid schedule local_date `{}`: {}. Use YYYY-MM-DD.",
            value, error
        )
    })
}

fn resolve_schedule_local_time(
    arguments: &serde_json::Value,
    context: ScheduleTaskScheduleContext<'_>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, String> {
    let Some(local_time) = schedule_text_field(arguments, "local_time") else {
        return Ok(None);
    };
    let time = parse_schedule_local_time(local_time)?;
    let timezone = ScheduleTaskTimezone::from_arguments(arguments, context.default_timezone)?;
    let date_policy = schedule_text_field(arguments, "date_policy")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let explicit_date = match schedule_text_field(arguments, "local_date") {
        Some(value) => Some(parse_schedule_local_date(value)?),
        None => None,
    };
    let now_local_date = timezone.local_date_for_utc(context.now_utc);
    let existing_local_date = context
        .existing_task_target
        .and_then(|task| task.scheduled_for)
        .map(|value| timezone.local_date_for_utc(value));
    let base_date = explicit_date.unwrap_or_else(|| {
        if date_policy == "next_occurrence" {
            now_local_date
        } else if date_policy == "same_local_date" {
            now_local_date
        } else {
            existing_local_date.unwrap_or(now_local_date)
        }
    });
    let mut scheduled_for = timezone.resolve_local_datetime(base_date.and_time(time))?;
    if explicit_date.is_none()
        && date_policy != "same_local_date"
        && scheduled_for <= context.now_utc
    {
        let next_date = now_local_date
            .succ_opt()
            .ok_or_else(|| "Unable to resolve the next local schedule date.".to_string())?;
        scheduled_for = timezone.resolve_local_datetime(next_date.and_time(time))?;
    }
    Ok(Some(scheduled_for))
}

fn schedule_task_schedule_from_arguments(
    arguments: &serde_json::Value,
    context: ScheduleTaskScheduleContext<'_>,
) -> Result<(Option<String>, Option<chrono::DateTime<chrono::Utc>>), String> {
    if let Some(cron) = schedule_text_field(arguments, "cron") {
        let cron_6field = if cron.split_whitespace().count() == 5 {
            format!("0 {}", cron)
        } else {
            cron.to_string()
        };
        return Ok((Some(cron_6field), None));
    }

    if let Some(at_time) = schedule_text_field(arguments, "at")
        .or_else(|| schedule_text_field(arguments, "scheduled_for"))
    {
        let dt = chrono::DateTime::parse_from_rfc3339(at_time).map_err(|error| {
            format!("Invalid schedule `at` timestamp `{}`: {}.", at_time, error)
        })?;
        return Ok((None, Some(dt.with_timezone(&chrono::Utc))));
    }

    if let Some(scheduled_for) = resolve_schedule_local_time(arguments, context)? {
        return Ok((None, Some(scheduled_for)));
    }

    Err("schedule_task requires `cron`, `at`, `scheduled_for`, or `local_time`; use `cron` for recurring work, an ISO timestamp for fully resolved one-time work, or local_time with timezone for wall-clock scheduling; refusing to infer the current time.".to_string())
}

fn schedule_task_validation_failure_result(detail: &str, reason: &str) -> String {
    render_tool_completion_marker_with_data(
        "schedule_task",
        "failed",
        detail,
        serde_json::json!({
            "success": false,
            "durable_commit": false,
            "durable_object": "Scheduled task",
            "reason": reason,
            "recoverable_by_model": true,
            "assistant_instruction": "Repair the schedule_task arguments from the current conversation context when the missing or invalid field is already available. For existing task updates, keep the same task_id and repair the schedule fields; do not cancel, delete, or recreate the task as an argument-repair path unless the user explicitly requested cancellation or deletion. Ask the user only when the required fact is absent."
        }),
    )
}

fn schedule_task_description_from_arguments(
    arguments: &serde_json::Value,
    existing_task_target: Option<&super::task::Task>,
) -> Option<String> {
    arguments
        .get("task")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            existing_task_target
                .as_ref()
                .map(|task| task.description.clone())
        })
        .or_else(|| {
            arguments
                .get("action_arguments")
                .and_then(|value| value.get("message"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

fn scheduled_task_uses_direct_notify_user_execution(task: &super::task::Task) -> bool {
    task.action.eq_ignore_ascii_case("notify_user")
}

fn scheduled_task_should_deliver_output_after_execution(task: &super::task::Task) -> bool {
    !scheduled_task_uses_direct_notify_user_execution(task)
}

fn scheduled_notify_user_execution_arguments(task: &super::task::Task) -> serde_json::Value {
    let mut payload = task.arguments.as_object().cloned().unwrap_or_default();
    let has_message = payload
        .get("message")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if !has_message {
        payload.insert(
            "message".to_string(),
            serde_json::Value::String(task.description.clone()),
        );
    }
    let has_delivery_channel = payload
        .get("delivery_channel")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if !has_delivery_channel {
        let route = normalize_automation_notification_channel(
            payload.get("report_to").and_then(|value| value.as_str()),
        );
        if !route.is_empty() {
            payload.insert(
                "delivery_channel".to_string(),
                serde_json::Value::String(route),
            );
        }
    }
    payload
        .entry("source".to_string())
        .or_insert_with(|| serde_json::Value::String("reminder".to_string()));
    payload
        .entry("in_app_title".to_string())
        .or_insert_with(|| serde_json::Value::String("Reminder".to_string()));
    serde_json::Value::Object(payload)
}

#[allow(clippy::too_many_arguments)]
fn task_automation_run_record(
    task: &super::task::Task,
    run_id: &str,
    status: AutomationRunStatus,
    attempt: u32,
    started_at: chrono::DateTime<chrono::Utc>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    origin: AutomationOriginContext,
    policy: AutomationExecutionPolicy,
    critique: AutomationCritique,
    output_preview: Option<String>,
    error: Option<String>,
    next_retry_at: Option<chrono::DateTime<chrono::Utc>>,
) -> AutomationRunRecord {
    AutomationRunRecord {
        id: run_id.to_string(),
        automation_id: task.id.to_string(),
        automation_kind: "task".to_string(),
        title: task.description.clone(),
        action: task.action.clone(),
        trigger: automation_trigger_label("scheduler", &task.action),
        status,
        attempt,
        started_at: started_at.to_rfc3339(),
        completed_at: completed_at.map(|value| value.to_rfc3339()),
        duration_ms: completed_at.map(|finished_at| {
            finished_at
                .signed_duration_since(started_at)
                .num_milliseconds()
                .max(0) as u64
        }),
        origin,
        policy,
        critique,
        output_preview,
        error,
        next_retry_at: next_retry_at.map(|value| value.to_rfc3339()),
    }
}

fn strip_tool_completion_marker_line(result: &str) -> String {
    let trimmed = result.trim_start();
    if trimmed
        .strip_prefix(crate::runtime::TOOL_COMPLETION_MARKER)
        .is_some()
    {
        trimmed.lines().skip(1).collect::<Vec<_>>().join("\n")
    } else {
        result.to_string()
    }
    .trim()
    .to_string()
}

fn task_action_is_settings_authorized_without_approval(action: &str) -> bool {
    action == "daily_brief"
}

fn remove_task_approval_envelope(arguments: &mut serde_json::Value) {
    if let serde_json::Value::Object(map) = arguments {
        map.remove("_approval");
    }
}

impl Agent {
    /// Add a task to the autonomous queue
    pub async fn add_task(&self, mut task: super::task::Task) -> Result<()> {
        task.approval = super::task::normalized_task_approval(&task.approval);
        if task_action_is_settings_authorized_without_approval(&task.action)
            && matches!(
                task.status,
                super::task::TaskStatus::AwaitingApproval
                    | super::task::TaskStatus::ExpiredNeedsReapproval
            )
        {
            task.status = super::task::TaskStatus::Pending;
            task.result = None;
            remove_task_approval_envelope(&mut task.arguments);
        }
        if super::task::task_requires_explicit_approval(&task.approval)
            && matches!(
                task.status,
                super::task::TaskStatus::Pending | super::task::TaskStatus::AwaitingApproval
            )
        {
            task.status = super::task::TaskStatus::AwaitingApproval;
        }
        let mut queue = self.tasks.write().await;
        self.storage.insert_task(&task).await?;
        queue.add(task.clone());
        drop(queue);
        if matches!(task.status, super::task::TaskStatus::AwaitingApproval) {
            self.register_task_approval_request(&task).await?;
        }
        Ok(())
    }

    pub async fn load_autonomy_settings(&self) -> super::autonomy::AutonomySettings {
        let mut settings = self
            .storage
            .get(AUTONOMY_SETTINGS_STORAGE_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_slice::<super::autonomy::AutonomySettings>(&raw).ok())
            .unwrap_or_default();
        settings.enforce_dependencies();
        settings
    }

    pub async fn save_autonomy_settings(
        &self,
        settings: &super::autonomy::AutonomySettings,
    ) -> std::result::Result<(), String> {
        let mut settings = settings.clone();
        settings.enforce_dependencies();
        let raw = serde_json::to_vec(&settings).map_err(|e| e.to_string())?;
        self.storage
            .set(AUTONOMY_SETTINGS_STORAGE_KEY, &raw)
            .await
            .map_err(|e| e.to_string())?;

        if super::autonomy::autonomy_background_paused(&settings) {
            let paused_since_missing = match self
                .storage
                .get(super::autonomy::AUTONOMY_PAUSED_SINCE_KEY)
                .await
            {
                Ok(Some(raw)) => String::from_utf8(raw)
                    .ok()
                    .map(|value| value.trim().is_empty())
                    .unwrap_or(true),
                Ok(None) => true,
                Err(error) => {
                    tracing::debug!(
                        "Failed to read autonomy pause state while saving settings: {}",
                        error
                    );
                    true
                }
            };
            if paused_since_missing {
                let now = chrono::Utc::now().timestamp().to_string();
                if let Err(error) = self
                    .storage
                    .set(super::autonomy::AUTONOMY_PAUSED_SINCE_KEY, now.as_bytes())
                    .await
                {
                    tracing::debug!(
                        "Failed to persist autonomy pause start while saving settings: {}",
                        error
                    );
                }
            }
        } else {
            let _ = self
                .storage
                .delete(super::autonomy::AUTONOMY_PAUSED_SINCE_KEY)
                .await;
            let _ = self
                .storage
                .delete(super::autonomy::AUTONOMY_PAUSE_NUDGE_LAST_SENT_AT_KEY)
                .await;
        }

        Ok(())
    }

    pub(super) async fn repair_unrecoverable_approval_tasks(&self) -> Result<usize> {
        let repair_reason =
            "Approval request was cleared because the original task details could not be restored after restart."
                .to_string();
        let repaired: Vec<(uuid::Uuid, super::task::Task)> = {
            let mut tasks = self.tasks.write().await;
            let ids = tasks
                .all()
                .iter()
                .filter(|task| {
                    matches!(
                        task.status,
                        super::task::TaskStatus::AwaitingApproval
                            | super::task::TaskStatus::ExpiredNeedsReapproval
                    ) && !task_has_actionable_approval_context(task)
                })
                .map(|task| task.id)
                .collect::<Vec<_>>();
            let mut repaired = Vec::new();
            for id in ids {
                if let Some(task) = tasks.get_mut(id) {
                    task.status = super::task::TaskStatus::Cancelled;
                    task.result = Some(repair_reason.clone());
                    repaired.push((id, task.clone()));
                }
            }
            repaired
        };

        for (id, task) in &repaired {
            let status_json =
                serde_json::to_string(&task.status).unwrap_or_else(|_| "\"Cancelled\"".to_string());
            let _ = self
                .storage
                .update_task_status_and_result(
                    &id.to_string(),
                    &status_json,
                    task.result.as_deref(),
                )
                .await;
            let _ = self
                .storage
                .resolve_approval_request(&id.to_string(), "expired", "startup_recovery")
                .await;
        }

        if !repaired.is_empty() {
            tracing::warn!(
                "Repaired {} unrecoverable approval task(s) with unavailable details",
                repaired.len()
            );
        }

        Ok(repaired.len())
    }

    pub async fn repair_settings_authorized_approval_tasks(&self) -> Result<usize> {
        let repaired: Vec<(uuid::Uuid, super::task::Task)> = {
            let mut tasks = self.tasks.write().await;
            let ids = tasks
                .all()
                .iter()
                .filter(|task| {
                    task_action_is_settings_authorized_without_approval(&task.action)
                        && matches!(
                            task.status,
                            super::task::TaskStatus::AwaitingApproval
                                | super::task::TaskStatus::ExpiredNeedsReapproval
                        )
                })
                .map(|task| task.id)
                .collect::<Vec<_>>();

            let mut repaired = Vec::new();
            for id in ids {
                if let Some(task) = tasks.get_mut(id) {
                    task.approval = super::task::TaskApproval::Auto;
                    task.status = super::task::TaskStatus::Pending;
                    task.result = None;
                    remove_task_approval_envelope(&mut task.arguments);
                    repaired.push((id, task.clone()));
                }
            }
            repaired
        };

        for (id, task) in &repaired {
            let arguments_json = serde_json::to_string(&task.arguments)?;
            let status_json =
                serde_json::to_string(&task.status).unwrap_or_else(|_| "\"Pending\"".to_string());
            if let Err(error) = self
                .storage
                .update_task(&id.to_string(), None, Some(arguments_json), None, None)
                .await
            {
                tracing::warn!(
                    task_id = %id,
                    error = %error,
                    "Failed to persist settings-authorized task approval cleanup"
                );
            }
            if let Err(error) = self
                .storage
                .update_task_status(&id.to_string(), &status_json)
                .await
            {
                tracing::warn!(
                    task_id = %id,
                    error = %error,
                    "Failed to persist settings-authorized task status cleanup"
                );
            }
            if let Err(error) = self
                .storage
                .resolve_approval_request(&id.to_string(), "stale", "settings_authorized_cleanup")
                .await
            {
                tracing::warn!(
                    task_id = %id,
                    error = %error,
                    "Failed to resolve settings-authorized task approval row"
                );
            }
        }

        Ok(repaired.len())
    }

    pub async fn register_task_approval_request(&self, task: &super::task::Task) -> Result<()> {
        let metadata = approval_metadata_from_arguments(&task.arguments).unwrap_or_else(|| {
            ApprovalRequestMetadata {
                title: task.description.clone(),
                summary: task.description.clone(),
                reason: String::new(),
                rule_name: "explicit_user_approval_required".to_string(),
                risk_level: String::new(),
                risk_score: None,
                source: String::new(),
            }
        });
        let arguments = serde_json::to_string(&task.arguments)?;
        self.encrypted_storage
            .upsert_approval_request_encrypted(
                &task.id.to_string(),
                &task.action,
                &arguments,
                &approval_rule_name_for_task(task, &metadata),
                &task.created_at.to_rfc3339(),
            )
            .await?;

        let body = approval_notification_text(task, &metadata);
        self.emit_notification("Approval Needed", &body, "warning", "approval")
            .await;
        self.notify_preferred_channel(&body).await;
        if let Err(error) = self
            .dispatch_plugin_event(
                "approval.requested",
                Self::approval_plugin_payload(task, &metadata),
            )
            .await
        {
            tracing::warn!(
                "Failed to dispatch plugin event approval.requested for task '{}': {}",
                task.id,
                error
            );
        }
        Ok(())
    }

    pub(super) fn record_task_approval_response(
        arguments: &mut serde_json::Value,
        decision: &str,
        resolved_by: &str,
        comment: Option<&str>,
    ) {
        if !arguments.is_object() {
            *arguments = serde_json::json!({});
        }
        let Some(obj) = arguments.as_object_mut() else {
            return;
        };
        obj.insert(
            "_approval_response".to_string(),
            serde_json::json!({
                "decision": decision,
                "resolved_by": resolved_by,
                "comment": comment.map(str::trim).filter(|value| !value.is_empty()),
                "resolved_at": chrono::Utc::now().to_rfc3339(),
            }),
        );
    }

    pub async fn approve_task_request(
        &self,
        id: uuid::Uuid,
        resolved_by: &str,
    ) -> Result<Option<super::task::Task>> {
        self.approve_task_request_with_comment(id, resolved_by, None)
            .await
    }

    pub async fn approve_task_request_with_comment(
        &self,
        id: uuid::Uuid,
        resolved_by: &str,
        comment: Option<&str>,
    ) -> Result<Option<super::task::Task>> {
        let updated_task = {
            let mut tasks = self.tasks.write().await;
            let Some(task) = tasks.get_mut(id) else {
                return Ok(None);
            };
            if !matches!(
                task.status,
                super::task::TaskStatus::AwaitingApproval
                    | super::task::TaskStatus::ExpiredNeedsReapproval
            ) {
                return Ok(None);
            }
            task.approval = super::task::normalized_task_approval(&task.approval);
            task.status = super::task::TaskStatus::Pending;
            task.result = None;
            Self::record_task_approval_response(
                &mut task.arguments,
                "approved",
                resolved_by,
                comment,
            );
            task.clone()
        };

        let status_json = serde_json::to_string(&updated_task.status)
            .unwrap_or_else(|_| "\"Pending\"".to_string());
        let arguments_json = serde_json::to_string(&updated_task.arguments)?;
        self.storage
            .update_task(&id.to_string(), None, Some(arguments_json), None, None)
            .await?;
        self.storage
            .update_task_status(&id.to_string(), &status_json)
            .await?;
        let _ = self
            .storage
            .resolve_approval_request(&id.to_string(), "approved", resolved_by)
            .await;
        Ok(Some(updated_task))
    }

    pub async fn reject_task_request(
        &self,
        id: uuid::Uuid,
        resolved_by: &str,
        reason: &str,
    ) -> Result<Option<super::task::Task>> {
        let resolved_reason = if reason.trim().is_empty() {
            "Task was rejected and will not be executed.".to_string()
        } else {
            reason.trim().to_string()
        };
        let updated_task = {
            let mut tasks = self.tasks.write().await;
            let Some(task) = tasks.get_mut(id) else {
                return Ok(None);
            };
            if !matches!(
                task.status,
                super::task::TaskStatus::AwaitingApproval
                    | super::task::TaskStatus::ExpiredNeedsReapproval
            ) {
                return Ok(None);
            }
            task.status = super::task::TaskStatus::Cancelled;
            task.result = Some(resolved_reason.clone());
            Self::record_task_approval_response(
                &mut task.arguments,
                "rejected",
                resolved_by,
                Some(&resolved_reason),
            );
            task.clone()
        };

        let status_json = serde_json::to_string(&updated_task.status)
            .unwrap_or_else(|_| "\"Cancelled\"".to_string());
        let arguments_json = serde_json::to_string(&updated_task.arguments)?;
        self.storage
            .update_task(&id.to_string(), None, Some(arguments_json), None, None)
            .await?;
        self.storage
            .update_task_status_and_result(&id.to_string(), &status_json, Some(&resolved_reason))
            .await?;
        let _ = self
            .storage
            .resolve_approval_request(&id.to_string(), "denied", resolved_by)
            .await;
        self.record_self_tune_user_rejection().await;
        Ok(Some(updated_task))
    }

    pub async fn expire_stale_approval_tasks_shared(
        storage: &Storage,
        tasks: &Arc<RwLock<TaskQueue>>,
        max_age_secs: i64,
    ) -> Result<usize> {
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(max_age_secs);
        let expired: Vec<(uuid::Uuid, super::task::Task)> = {
            let mut tasks = tasks.write().await;
            let ids = tasks
                .all()
                .iter()
                .filter(|task| {
                    matches!(task.status, super::task::TaskStatus::AwaitingApproval)
                        && task.created_at < cutoff
                })
                .map(|task| task.id)
                .collect::<Vec<_>>();
            let mut expired = Vec::new();
            for id in &ids {
                if let Some(task) = tasks.get_mut(*id) {
                    task.status = super::task::TaskStatus::ExpiredNeedsReapproval;
                    task.result = Some(
                        "Approval expired and now needs reapproval before the task can continue."
                            .to_string(),
                    );
                    expired.push((*id, task.clone()));
                }
            }
            expired
        };

        for (id, task) in &expired {
            let status_json = serde_json::to_string(&task.status)
                .unwrap_or_else(|_| "\"ExpiredNeedsReapproval\"".to_string());
            let _ = storage
                .update_task_status_and_result(
                    &id.to_string(),
                    &status_json,
                    task.result.as_deref(),
                )
                .await;
            let _ = storage
                .resolve_approval_request(&id.to_string(), "expired", "auto_timeout")
                .await;
        }

        Ok(expired.len())
    }

    pub(super) async fn recover_stale_in_progress_tasks(&self) {
        let states = match super::automation::list_supervisor_states(&self.storage).await {
            Ok(states) => states,
            Err(error) => {
                tracing::debug!(
                    "Failed to load automation supervisor states for recovery: {}",
                    error
                );
                return;
            }
        };
        if states.is_empty() {
            return;
        }

        let state_map: HashMap<String, AutomationSupervisorState> = states
            .into_iter()
            .map(|state| (state.automation_id.clone(), state))
            .collect();
        let now = chrono::Utc::now();
        let mut recovered: Vec<(String, serde_json::Value, Option<String>, Option<String>)> =
            Vec::new();
        let mut updated_states: Vec<AutomationSupervisorState> = Vec::new();

        {
            let mut tasks = self.tasks.write().await;
            let snapshot = tasks.all().to_vec();
            for task in snapshot {
                if !matches!(task.status, super::task::TaskStatus::InProgress) {
                    continue;
                }
                let Some(state) = state_map.get(&task.id.to_string()).cloned() else {
                    continue;
                };
                let last_run_at = state
                    .last_run_at
                    .as_deref()
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.with_timezone(&chrono::Utc));
                let policy = automation_policy_from_arguments(
                    &task.arguments,
                    default_automation_validation_for_action(&task.action),
                );
                let stale = match last_run_at {
                    Some(last) => {
                        now.signed_duration_since(last)
                            > chrono::Duration::seconds(
                                (policy.stall_timeout_secs.saturating_mul(2)) as i64,
                            )
                    }
                    None => true,
                };
                if !stale {
                    continue;
                }

                if let Some(entry) = tasks.get_mut(task.id) {
                    entry.status = super::task::TaskStatus::Pending;
                    entry.scheduled_for = Some(now);
                    entry.result = Some(
                        "Recovered after a stale in-progress background run was detected."
                            .to_string(),
                    );
                    recovered.push((
                        entry.id.to_string(),
                        entry.arguments.clone(),
                        entry.cron.clone(),
                        entry.scheduled_for.as_ref().map(|dt| dt.to_rfc3339()),
                    ));
                }

                let mut next_state = state.clone();
                next_state.status = "stalled".to_string();
                next_state.stalled_count = next_state.stalled_count.saturating_add(1);
                next_state.last_error = Some(
                    "Recovered after stale in-progress run exceeded stall timeout.".to_string(),
                );
                next_state.next_retry_at = Some(now.to_rfc3339());
                updated_states.push(next_state);
            }
        }

        for (id, arguments, cron, scheduled_for) in recovered {
            let args_json = serde_json::to_string(&arguments).ok();
            if let Err(error) = self
                .storage
                .update_task_status_and_result(
                    &id,
                    &serde_json::to_string(&super::task::TaskStatus::Pending)
                        .unwrap_or_else(|_| "\"Pending\"".to_string()),
                    Some("Recovered after stale in-progress background run was detected."),
                )
                .await
            {
                tracing::warn!(
                    "Failed to persist stale task recovery status for '{}': {}",
                    id,
                    error
                );
            }
            if let Err(error) = self
                .storage
                .update_task(&id, None, args_json, cron, scheduled_for)
                .await
            {
                tracing::warn!(
                    "Failed to persist stale task recovery schedule for '{}': {}",
                    id,
                    error
                );
            }
        }
        for state in updated_states {
            if let Err(error) = upsert_automation_supervisor_state(&self.storage, state).await {
                tracing::warn!(
                    "Failed to persist recovered automation supervisor state: {}",
                    error
                );
            }
        }
    }

    /// Take due tasks and mark them in-progress
    pub async fn take_due_tasks(&self, reminders_only: bool) -> Vec<super::task::Task> {
        self.recover_stale_in_progress_tasks().await;
        let now = chrono::Utc::now();
        let mut due = Vec::new();
        let mut claim_candidates: Vec<super::task::Task> = Vec::new();
        let mut expired_reminders: Vec<(String, String)> = Vec::new();
        let mut schedule_updates: Vec<(String, Option<String>, Option<String>)> = Vec::new();
        let tz = {
            let profile = self.user_profile.read().await;
            profile
                .timezone
                .as_deref()
                .and_then(|value| value.parse::<chrono_tz::Tz>().ok())
        };

        {
            let mut tasks = self.tasks.write().await;
            let snapshot = tasks.all().to_vec();
            for task in snapshot.iter() {
                let mut should_run = false;
                let mut next_run: Option<chrono::DateTime<chrono::Utc>> = None;

                if matches!(task.status, super::task::TaskStatus::Pending) {
                    if let Some(ref cron) = task.cron {
                        // If no scheduled_for, compute next run
                        if task.scheduled_for.is_none() {
                            let task_tz = if task.action == "daily_brief" {
                                tz
                            } else {
                                None
                            };
                            next_run = compute_next_run(cron, task_tz);
                        } else if let Some(sf) = task.scheduled_for {
                            if sf <= now {
                                should_run = true;
                            }
                        }
                    } else if let Some(at) = task.scheduled_for {
                        if at <= now {
                            should_run = true;
                        }
                    } else {
                        should_run = true;
                    }
                }

                if let Some(nr) = next_run {
                    if let Some(t) = tasks.get_mut(task.id) {
                        t.scheduled_for = Some(nr);
                        schedule_updates.push((
                            t.id.to_string(),
                            t.cron.clone(),
                            t.scheduled_for.as_ref().map(|d| d.to_rfc3339()),
                        ));
                    }
                }

                if should_run {
                    if super::task::one_shot_reminder_is_expired(task, now) {
                        if let Some(entry) = tasks.get_mut(task.id) {
                            let result = Self::expired_one_shot_reminder_result(entry, now);
                            entry.status = super::task::TaskStatus::Cancelled;
                            entry.result = Some(result.clone());
                            expired_reminders.push((entry.id.to_string(), result));
                        }
                        continue;
                    }
                    let is_reminder = super::task::task_is_scheduled_reminder(task);
                    if !reminders_only || is_reminder {
                        claim_candidates.push(task.clone());
                    }
                }
            }
        }

        for (id, cron, scheduled_for) in schedule_updates {
            if let Err(error) = self
                .storage
                .update_task(&id, None, None, cron, scheduled_for)
                .await
            {
                tracing::warn!(
                    "Failed to persist scheduled task update for '{}': {}",
                    id,
                    error
                );
            }
        }

        let cancelled_status = serde_json::to_string(&super::task::TaskStatus::Cancelled)
            .unwrap_or_else(|_| "\"Cancelled\"".to_string());
        for (id, result) in expired_reminders {
            if let Err(error) = self
                .storage
                .update_task_status_and_result(&id, &cancelled_status, Some(&result))
                .await
            {
                tracing::warn!(
                    "Failed to persist expired reminder status for '{}': {}",
                    id,
                    error
                );
            }
        }

        let pending_status = serde_json::to_string(&super::task::TaskStatus::Pending)
            .unwrap_or_else(|_| "\"Pending\"".to_string());
        let in_progress_status = serde_json::to_string(&super::task::TaskStatus::InProgress)
            .unwrap_or_else(|_| "\"InProgress\"".to_string());
        let lease_owner = format!("pid:{}:{}", std::process::id(), self.identity.did());
        claim_candidates.sort_by(super::task::task_due_priority_cmp);

        for task in claim_candidates {
            let policy = automation_policy_from_arguments(
                &task.arguments,
                default_automation_validation_for_action(&task.action),
            );
            let lease_expires_at = (chrono::Utc::now()
                + chrono::Duration::seconds(
                    policy.stall_timeout_secs.max(300).saturating_mul(2) as i64
                ))
            .to_rfc3339();
            let claimed = match self
                .storage
                .try_claim_task(
                    &task.id.to_string(),
                    &pending_status,
                    &in_progress_status,
                    &lease_owner,
                    &lease_expires_at,
                )
                .await
            {
                Ok(claimed) => claimed,
                Err(error) => {
                    tracing::warn!("Failed to claim due task '{}': {}", task.id, error);
                    false
                }
            };
            if !claimed {
                continue;
            }

            if let Some(claimed_task) = {
                let mut tasks = self.tasks.write().await;
                if let Some(entry) = tasks.get_mut(task.id) {
                    entry.status = super::task::TaskStatus::InProgress;
                    Some(entry.clone())
                } else {
                    None
                }
            } {
                due.push(claimed_task);
            } else {
                tracing::warn!(
                    "Task '{}' was claimed in Postgres but missing from in-memory queue snapshot",
                    task.id
                );
            }
        }

        due
    }

    pub(super) fn reminder_lateness_label(lateness: chrono::Duration) -> String {
        let total_seconds = lateness.num_seconds().max(0);
        if total_seconds < 60 {
            return format!("{} seconds", total_seconds);
        }
        if total_seconds < 3600 {
            let minutes = (total_seconds + 59) / 60;
            return format!("{} minute{}", minutes, if minutes == 1 { "" } else { "s" });
        }
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600 + 59) / 60;
        if minutes == 0 {
            format!("{} hour{}", hours, if hours == 1 { "" } else { "s" })
        } else {
            format!(
                "{} hour{} {} minute{}",
                hours,
                if hours == 1 { "" } else { "s" },
                minutes,
                if minutes == 1 { "" } else { "s" }
            )
        }
    }

    pub(super) fn expired_one_shot_reminder_result(
        task: &super::task::Task,
        now: chrono::DateTime<chrono::Utc>,
    ) -> String {
        if let (Some(scheduled_for), Some(lateness)) =
            (task.scheduled_for, super::task::task_lateness(task, now))
        {
            return format!(
                "Skipped sending this reminder because it was {} late, which exceeds the 15 minute delivery window. Scheduled for {}.",
                Self::reminder_lateness_label(lateness),
                scheduled_for.to_rfc3339()
            );
        }
        "Skipped sending this reminder because it missed the 15 minute delivery window.".to_string()
    }

    pub(super) fn delayed_reminder_notice(
        task: &super::task::Task,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<String> {
        if !super::task::one_shot_reminder_needs_delay_notice(task, now) {
            return None;
        }
        let lateness = super::task::task_lateness(task, now)?;
        Some(format!(
            "Late reminder: AgentArk was unavailable at the scheduled time, so this is being sent now ({} late).",
            Self::reminder_lateness_label(lateness)
        ))
    }

    pub(super) fn prepend_delayed_reminder_notice(
        task: &super::task::Task,
        message: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> String {
        if let Some(notice) = Self::delayed_reminder_notice(task, now) {
            format!("{}\n\n{}", notice, message.trim())
        } else {
            message.to_string()
        }
    }

    pub(super) fn notify_user_arguments_with_delay_notice(
        task: &super::task::Task,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<serde_json::Value> {
        let notice = Self::delayed_reminder_notice(task, now)?;
        let mut payload = task.arguments.as_object()?.clone();
        let message = payload
            .get("message")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        payload.insert(
            "message".to_string(),
            serde_json::Value::String(format!("{}\n\n{}", notice, message)),
        );
        Some(serde_json::Value::Object(payload))
    }

    pub(super) async fn execute_workflow_marker_action(
        &self,
        action_name: &str,
        user_query: &str,
    ) -> Result<String> {
        if let Some(workflow_content) = self.runtime.get_workflow_content(action_name).await {
            self.runtime
                .execute_workflow_action(action_name, &workflow_content, user_query, &self.llm)
                .await
        } else {
            Ok(format!(
                "Workflow content not found for action: {}",
                action_name
            ))
        }
    }

    pub(super) fn format_missing_inputs_prompt(payload: &WorkflowMissingInputsPayload) -> String {
        let missing = if payload.missing.is_empty() {
            "required fields".to_string()
        } else {
            payload
                .missing
                .iter()
                .map(|f| format!("`{}`", f))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let sensitive_like = payload.sensitive_missing.clone();

        if sensitive_like.is_empty() {
            format!(
                "I need a bit more information to run `{}`.\nMissing input(s): {}.\nPlease provide these values and run again.",
                payload.action, missing
            )
        } else {
            let sensitive_list = sensitive_like
                .iter()
                .map(|k| format!("`{}`", k))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "I need your confirmation before I continue with `{}`.\nMissing input(s): {}\nSensitive key(s): {}\n\nUse the secure credential form in chat or Settings to provide sensitive values. Sensitive values are stored encrypted and handled outside model generation for safety.",
                payload.action, missing, sensitive_list
            )
        }
    }

    pub(super) fn build_scheduled_input_needed_result(
        payload: &WorkflowMissingInputsPayload,
    ) -> ScheduledInputNeededResult {
        let missing = if payload.missing.is_empty() {
            vec!["required fields".to_string()]
        } else {
            payload.missing.clone()
        };
        let required = if payload.required.is_empty() {
            vec!["required fields".to_string()]
        } else {
            payload.required.clone()
        };
        let provided = payload.provided.clone();
        let missing_label = missing
            .iter()
            .map(|value| format!("`{}`", value))
            .collect::<Vec<_>>()
            .join(", ");
        let required_label = required
            .iter()
            .map(|value| format!("`{}`", value))
            .collect::<Vec<_>>()
            .join(", ");
        let sensitive_like = payload.sensitive_missing.clone();
        let fix_hint = if sensitive_like.is_empty() {
            "Open the task, add the missing fields, then resume it.".to_string()
        } else {
            let sensitive_label = sensitive_like
                .iter()
                .map(|value| format!("`{}`", value))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Add the missing secret values in Settings -> Secrets, then resume the task. Sensitive keys: {}.",
                sensitive_label
            )
        };
        let summary = format!(
            "Scheduled action '{}' paused because required inputs are missing.",
            payload.action
        );
        let notification_body = if provided.is_empty() {
            format!(
                "{}\nMissing fields: {}\nRequired fields: {}\nFix: {}",
                summary, missing_label, required_label, fix_hint
            )
        } else {
            let provided_label = provided
                .iter()
                .map(|value| format!("`{}`", value))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{}\nMissing fields: {}\nRequired fields: {}\nProvided fields: {}\nFix: {}",
                summary, missing_label, required_label, provided_label, fix_hint
            )
        };

        ScheduledInputNeededResult {
            kind: "input_needed".to_string(),
            action: payload.action.clone(),
            query: payload.query.clone(),
            missing,
            required,
            provided,
            summary,
            fix_hint,
            notification_title: "Input needed".to_string(),
            notification_body,
        }
    }

    pub(super) async fn run_scheduled_fallback_for_missing_inputs(
        &self,
        payload: &WorkflowMissingInputsPayload,
    ) -> Result<String> {
        let result = Self::build_scheduled_input_needed_result(payload);
        self.emit_notification(
            &result.notification_title,
            &result.notification_body,
            "warning",
            "workflow_inputs",
        )
        .await;

        Ok(format!(
            "{}{}",
            SCHEDULED_INPUT_NEEDED_MARKER,
            serde_json::to_string(&result)?
        ))
    }

    pub(super) fn parse_scheduled_input_needed_result(
        output: &str,
    ) -> Option<ScheduledInputNeededResult> {
        let payload = output.strip_prefix(SCHEDULED_INPUT_NEEDED_MARKER)?;
        serde_json::from_str::<ScheduledInputNeededResult>(payload).ok()
    }

    pub(super) fn watcher_condition_signature(
        condition: &crate::core::watcher::WatchCondition,
    ) -> serde_json::Value {
        serde_json::to_value(condition).unwrap_or_else(|_| serde_json::json!({"invalid": true}))
    }

    pub(super) fn watcher_runtime_signature(watcher: &crate::core::watcher::Watcher) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            watcher.description,
            watcher.poll_action,
            watcher.poll_arguments,
            watcher.interval_secs,
            watcher.timeout_secs,
            watcher.on_trigger,
            watcher.notify_channel,
            Self::watcher_condition_signature(&watcher.condition)
        )
    }

    pub async fn apply_autopilot_mode(
        &self,
        settings: &mut super::autonomy::AutonomySettings,
        mode_id: &str,
    ) -> std::result::Result<serde_json::Value, String> {
        let Some(mode) = settings.modes.iter().find(|m| m.id == mode_id).cloned() else {
            return Err("Mode not found".to_string());
        };

        let available_actions: HashSet<String> = self
            .runtime
            .list_enabled_actions()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|a| a.name)
            .collect();

        let mut routines_created = 0usize;
        let mut watchers_created = 0usize;
        let mut skipped: Vec<String> = Vec::new();
        let existing_tasks = {
            let tasks = self.tasks.read().await;
            tasks.all().to_vec()
        };
        let existing_watchers = self.watcher_manager.list().await;
        let approval_key = |approval: &super::task::TaskApproval| match approval {
            super::task::TaskApproval::RequireApproval => "require",
            super::task::TaskApproval::NotifyThenExecute { .. } => "require",
            super::task::TaskApproval::Auto => "auto",
        };
        let mut seen_routine_signatures: HashSet<String> = existing_tasks
            .iter()
            .filter(|task| {
                matches!(
                    task.status,
                    super::task::TaskStatus::Pending
                        | super::task::TaskStatus::AwaitingApproval
                        | super::task::TaskStatus::Paused
                        | super::task::TaskStatus::InProgress
                )
            })
            .map(|task| {
                format!(
                    "{}|{}|{}|{}|{}",
                    task.description,
                    task.action,
                    task.arguments,
                    task.cron.clone().unwrap_or_default(),
                    approval_key(&task.approval)
                )
            })
            .collect();
        let mut seen_watcher_signatures: HashSet<String> = existing_watchers
            .iter()
            .filter(|watcher| watcher.status == crate::core::watcher::WatcherStatus::Active)
            .map(Self::watcher_runtime_signature)
            .collect();

        for routine in &mode.routines {
            let builtin_action = routine.action == "daily_brief"
                || routine.action == "goal_progress_report"
                || routine.action == "plan";
            if !builtin_action && !available_actions.contains(&routine.action) {
                skipped.push(format!(
                    "routine '{}' skipped (missing action '{}')",
                    routine.description, routine.action
                ));
                continue;
            }
            let desired_approval_key = match routine.approval.as_deref().unwrap_or("auto") {
                "require" | "require_approval" | "notify" | "notify_then_execute" => "require",
                _ => "auto",
            };
            let routine_signature = format!(
                "{}|{}|{}|{}|{}",
                routine.description,
                routine.action,
                routine.arguments,
                routine.cron.clone().unwrap_or_default(),
                desired_approval_key
            );
            if seen_routine_signatures.contains(&routine_signature) {
                skipped.push(format!("routine '{}' already exists", routine.description));
                continue;
            }
            let mut task = super::task::Task::new(
                routine.description.clone(),
                routine.action.clone(),
                routine.arguments.clone(),
            );
            task.cron = routine.cron.clone();
            task.approval = match routine.approval.as_deref().unwrap_or("auto") {
                "require" | "require_approval" | "notify" | "notify_then_execute" => {
                    super::task::TaskApproval::RequireApproval
                }
                _ => super::task::TaskApproval::Auto,
            };
            task.status = super::task::status_for_task_approval(&task.approval);
            if let Err(e) = self.add_task(task).await {
                skipped.push(format!("routine '{}' failed: {}", routine.description, e));
                continue;
            }
            seen_routine_signatures.insert(routine_signature);
            routines_created += 1;
        }

        for watcher in &mode.watchers {
            if !available_actions.contains(&watcher.poll_action) {
                skipped.push(format!(
                    "watcher '{}' skipped (missing poll action '{}')",
                    watcher.description, watcher.poll_action
                ));
                continue;
            }
            let desired_condition =
                serde_json::to_value(&watcher.condition).unwrap_or_else(|_| serde_json::json!({}));
            let watcher_signature = format!(
                "{}|{}|{}|{}|{}|{}|{}|{}",
                watcher.description,
                watcher.poll_action,
                watcher.poll_arguments,
                watcher.interval_secs,
                watcher.timeout_secs,
                watcher.on_trigger,
                watcher.notify_channel,
                desired_condition
            );
            if seen_watcher_signatures.contains(&watcher_signature) {
                skipped.push(format!("watcher '{}' already exists", watcher.description));
                continue;
            }

            let arguments = serde_json::json!({
                "description": watcher.description,
                "poll_action": watcher.poll_action,
                "poll_arguments": watcher.poll_arguments,
                "interval_secs": watcher.interval_secs,
                "timeout_secs": watcher.timeout_secs,
                "on_trigger": watcher.on_trigger,
                "notify_channel": watcher.notify_channel,
                "condition": watcher.condition,
            });
            match self
                .handle_watch(&arguments, "autonomy", None, None, None)
                .await
            {
                Some(message) => {
                    let watcher_applied = self
                        .watcher_manager
                        .list()
                        .await
                        .into_iter()
                        .filter(|watcher| {
                            watcher.status == crate::core::watcher::WatcherStatus::Active
                        })
                        .map(|watcher| Self::watcher_runtime_signature(&watcher))
                        .any(|signature| signature == watcher_signature);
                    if watcher_applied {
                        seen_watcher_signatures.insert(watcher_signature);
                        watchers_created += 1;
                    } else {
                        skipped.push(message);
                    }
                }
                None => skipped.push(format!(
                    "watcher '{}' failed to create",
                    watcher.description
                )),
            }
        }

        settings.active_mode_id = Some(mode.id.clone());
        self.save_autonomy_settings(settings).await?;

        Ok(serde_json::json!({
            "status": "ok",
            "mode_id": mode.id,
            "mode_name": mode.name,
            "routines_created": routines_created,
            "watchers_created": watchers_created,
            "skipped": skipped,
        }))
    }

    pub async fn execute_autonomy_action_payload(
        &self,
        settings: &mut super::autonomy::AutonomySettings,
        action_kind: &str,
        payload: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        match action_kind {
            "daily_brief_now" => {
                let result = self
                    .run_daily_brief_reported_with_hint(None)
                    .await
                    .map_err(|e| e.to_string())?;
                let delivered_channel = result
                    .push_attempts
                    .iter()
                    .find(|outcome| {
                        outcome.success && is_external_notification_channel(&outcome.channel)
                    })
                    .map(|outcome| outcome.channel.clone());
                let summarize_outcome = |outcome: &NotificationDispatchOutcome| {
                    serde_json::json!({
                        "channel": outcome.channel,
                        "success": outcome.success,
                        "error": outcome.error.as_deref().map(crate::security::redact_pii),
                        "delivery": outcome.delivery.as_str(),
                    })
                };
                Ok(serde_json::json!({
                    "status":"executed",
                    "kind":"daily_brief_now",
                    "brief": crate::security::redact_pii(&result.brief),
                    "delivery": {
                        "in_app_notification_suppressed": true,
                        "stored_in_app": result.in_app.success,
                        "in_app": summarize_outcome(&result.in_app),
                        "push_delivered": delivered_channel.is_some(),
                        "delivered_channel": delivered_channel,
                        "push_attempts": result
                            .push_attempts
                            .iter()
                            .map(summarize_outcome)
                            .collect::<Vec<_>>(),
                    }
                }))
            }
            "chat_prompt" => {
                let prompt = payload.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
                let channel = payload
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("autonomy");
                let conversation_id = payload
                    .get("conversation_id")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.trim().is_empty());
                let project_id = payload
                    .get("project_id")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.trim().is_empty());
                let response = self
                    .process_message_with_meta(prompt, channel, conversation_id, project_id)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::json!({
                    "status":"executed",
                    "kind":"chat_prompt",
                    "conversation_id": response.conversation_id,
                    "trace_id": response.trace_id,
                    "response": crate::security::redact_pii(&response.response),
                }))
            }
            "create_task" => {
                let description = payload
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Autonomy task");
                let action_name = payload
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("daily_brief");
                let request_channel = payload
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("autonomy");
                let conversation_id = payload
                    .get("conversation_id")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.trim().is_empty());
                let project_id = payload
                    .get("project_id")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.trim().is_empty());
                let mut arguments = payload
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                let explicit_background_session_id =
                    super::background_session::background_session_id_from_automation(&arguments);
                let action_policy = self
                    .runtime
                    .list_enabled_actions()
                    .await
                    .map(|actions| background_session_policy_for_action(&actions, action_name))
                    .unwrap_or_default();
                let background_session = self
                    .ensure_background_session_for_automation(
                        request_channel,
                        conversation_id,
                        project_id,
                        explicit_background_session_id.as_deref(),
                        description,
                        "Wait for the created task to run and capture the result.",
                        action_policy,
                    )
                    .await;
                let origin = AutomationOriginContext {
                    channel: Some(request_channel.to_string()),
                    conversation_id: conversation_id.map(|value| value.to_string()),
                    project_id: project_id.map(|value| value.to_string()),
                    source: Some("autonomy_create_task".to_string()),
                };
                arguments = inject_automation_context(
                    &arguments,
                    origin,
                    AutomationExecutionPolicy::default(),
                );
                arguments = inject_automation_authorization_context(&arguments, None);
                let background_session_id = background_session
                    .as_ref()
                    .map(|session| session.id.as_str())
                    .or(explicit_background_session_id.as_deref());
                arguments = super::background_session::set_background_session_id_in_automation(
                    &arguments,
                    background_session_id,
                );
                let mut task = super::task::Task::new(
                    description.to_string(),
                    action_name.to_string(),
                    arguments,
                );
                task.approval = match payload
                    .get("approval")
                    .and_then(|v| v.as_str())
                    .unwrap_or("auto")
                {
                    "require" | "require_approval" | "notify" | "notify_then_execute" => {
                        super::task::TaskApproval::RequireApproval
                    }
                    _ => super::task::TaskApproval::Auto,
                };
                task.status = super::task::status_for_task_approval(&task.approval);
                if let Some(cron) = payload.get("cron").and_then(|v| v.as_str()) {
                    task.cron = Some(cron.to_string());
                }
                let allow_duplicate = payload
                    .get("allow_duplicate")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let (task_id, reused_existing, removed_duplicates) = self
                    .add_or_update_similar_task(task, allow_duplicate, None)
                    .await
                    .map_err(|e| e.to_string())?;
                if let Some(session_id) = background_session_id {
                    self.attach_items_to_background_session(
                        session_id,
                        &[task_id.to_string()],
                        &[],
                    )
                    .await;
                }
                Ok(serde_json::json!({
                    "status":"executed",
                    "kind":"create_task",
                    "task_id": task_id,
                    "reused_existing": reused_existing,
                    "removed_duplicates": removed_duplicates,
                }))
            }
            "watch" => {
                let request_channel = payload
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("autonomy");
                let conversation_id = payload
                    .get("conversation_id")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.trim().is_empty());
                let project_id = payload
                    .get("project_id")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.trim().is_empty());
                let output = self
                    .handle_watch(payload, request_channel, conversation_id, project_id, None)
                    .await
                    .ok_or_else(|| "Watcher creation did not return a result".to_string())?;
                let data = schedule_task_completion_data(&output);
                let watcher_id = data
                    .as_ref()
                    .and_then(|value| value.get("watcher_id"))
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string());
                Ok(serde_json::json!({
                    "status": "executed",
                    "kind": "watch",
                    "watcher_id": watcher_id,
                    "watcher": data,
                    "message": strip_tool_completion_marker_line(&output),
                }))
            }
            "activate_mode" => {
                let mode_id = payload
                    .get("mode_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let info = self.apply_autopilot_mode(settings, mode_id).await?;
                Ok(serde_json::json!({"status":"executed","kind":"activate_mode","result":info}))
            }
            "delegate" => {
                let task = payload.get("task").and_then(|v| v.as_str()).unwrap_or("");
                if task.trim().is_empty() {
                    return Err("Delegation payload missing task".to_string());
                }
                if let Some(ref swarm) = self.swarm {
                    let context = payload
                        .get("context")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let mut actions = self
                        .runtime
                        .list_enabled_actions()
                        .await
                        .unwrap_or_default();
                    self.append_dynamic_integration_actions(&mut actions).await;
                    let active_specialist_prompt_bundle =
                        self.active_specialist_prompt_bundle_for_message(task).await;
                    let result = swarm
                        .delegate(
                            task,
                            context,
                            &self.llm,
                            &[],
                            &actions,
                            Some(&active_specialist_prompt_bundle),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    let delegation = crate::storage::entities::swarm_delegation::Model {
                        id: uuid::Uuid::new_v4().to_string(),
                        parent_task_id: None,
                        agent_id: result.agents_used.join(","),
                        task_description: task.to_string(),
                        result: Some(result.final_result.clone()),
                        success: 1,
                        confidence: Some(0.8),
                        execution_time_ms: Some(result.total_time_ms.min(i32::MAX as u64) as i32),
                        created_at: chrono::Utc::now().to_rfc3339(),
                        completed_at: Some(chrono::Utc::now().to_rfc3339()),
                    };
                    let _ = self.storage.insert_swarm_delegation(&delegation).await;
                    return Ok(serde_json::json!({
                        "status":"executed",
                        "kind":"delegate",
                        "final_result": crate::security::redact_pii(&result.final_result),
                        "agents_used": result.agents_used,
                        "total_time_ms": result.total_time_ms,
                    }));
                }
                Err("Swarm is not enabled".to_string())
            }
            other => Err(format!("Unsupported autonomy action '{}'", other)),
        }
    }

    /// Execute a task through the model-routed spine.
    pub async fn execute_task(&self, task: &super::task::Task) -> Result<String> {
        if scheduled_task_uses_direct_notify_user_execution(task) {
            let arguments = scheduled_notify_user_execution_arguments(task);
            return self.execute_direct_notify_user_tool(&arguments).await;
        }
        self.run_model_routed_spine_for_task(task).await
    }

    /// Update task result and status.
    pub async fn finalize_task(
        &self,
        id: uuid::Uuid,
        status: super::task::TaskStatus,
        result: Option<String>,
    ) -> Result<()> {
        let mut stored_status = status.clone();
        let mut schedule_update: Option<(Option<String>, Option<String>)> = None;
        let tz = {
            let profile = self.user_profile.read().await;
            profile
                .timezone
                .as_deref()
                .and_then(|value| value.parse::<chrono_tz::Tz>().ok())
        };

        {
            let mut tasks = self.tasks.write().await;
            if let Some(task) = tasks.get_mut(id) {
                if should_preserve_cancelled_task_status(&task.status, &status) {
                    return Ok(());
                }
                if task.cron.is_some()
                    && matches!(
                        status,
                        super::task::TaskStatus::Completed | super::task::TaskStatus::Failed { .. }
                    )
                {
                    let task_tz = if task.action == "daily_brief" {
                        tz
                    } else {
                        None
                    };
                    task.scheduled_for = task
                        .cron
                        .as_deref()
                        .and_then(|cron| compute_next_run(cron, task_tz));
                    stored_status = super::task::TaskStatus::Pending;
                }
                task.status = stored_status.clone();
                task.result = result.clone();
                if task.cron.is_some() {
                    schedule_update = Some((
                        task.cron.clone(),
                        task.scheduled_for.as_ref().map(|d| d.to_rfc3339()),
                    ));
                }
            }
        }

        let status_json =
            serde_json::to_string(&stored_status).unwrap_or_else(|_| "Completed".to_string());
        self.storage
            .update_task_status_and_result(&id.to_string(), &status_json, result.as_deref())
            .await?;

        if let Some((cron, scheduled_for)) = schedule_update {
            let _ = self
                .storage
                .update_task(&id.to_string(), None, None, cron, scheduled_for)
                .await;
        }

        Ok(())
    }

    pub(super) async fn reschedule_task_retry(
        &self,
        task: &super::task::Task,
        next_retry_at: chrono::DateTime<chrono::Utc>,
        result: &str,
        attempt: u32,
    ) -> Result<()> {
        let mut updated_arguments = task.arguments.clone();
        automation_increment_attempt(&mut updated_arguments, attempt + 1);
        {
            let mut tasks = self.tasks.write().await;
            if let Some(entry) = tasks.get_mut(task.id) {
                entry.status = super::task::TaskStatus::Pending;
                entry.scheduled_for = Some(next_retry_at);
                entry.result = Some(result.to_string());
                entry.arguments = updated_arguments.clone();
            }
        }
        let args_json = serde_json::to_string(&updated_arguments)?;
        self.storage
            .update_task_status_and_result(
                &task.id.to_string(),
                &serde_json::to_string(&super::task::TaskStatus::Pending)
                    .unwrap_or_else(|_| "\"Pending\"".to_string()),
                Some(result),
            )
            .await?;
        self.storage
            .update_task(
                &task.id.to_string(),
                None,
                Some(args_json),
                task.cron.clone(),
                Some(next_retry_at.to_rfc3339()),
            )
            .await?;
        Ok(())
    }

    pub(super) fn task_report_target(task: &super::task::Task) -> String {
        normalize_automation_notification_channel(
            task.arguments
                .get("report_to")
                .and_then(|value| value.as_str()),
        )
    }

    pub(super) fn task_report_is_chat_owned(task: &super::task::Task) -> bool {
        let origin = task
            .arguments
            .get("_origin")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or_default();
        if !origin.eq_ignore_ascii_case("chat") {
            return false;
        }

        let task_kind = task
            .arguments
            .get("_task_kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or_default();
        task_kind.eq_ignore_ascii_case("chat_request")
            || task.action.eq_ignore_ascii_case("chat_request")
    }

    pub(super) async fn preferred_task_report_channel_hint(
        &self,
        task: &super::task::Task,
    ) -> Option<String> {
        if Self::task_report_target(task) != "preferred" {
            return None;
        }
        let session_id =
            super::background_session::background_session_id_from_automation(&task.arguments);
        if let Some(session_id) = session_id.as_deref() {
            if let Some(session) = self.background_sessions.get(session_id).await {
                if let Some(preferred) = session.preferred_delivery_channel.as_deref() {
                    let normalized = preferred.trim().to_ascii_lowercase();
                    if !normalized.is_empty()
                        && (!is_external_notification_channel(&normalized)
                            || self
                                .notification_channel_is_configured_any(&normalized)
                                .await)
                    {
                        return Some(normalized);
                    }
                }
            }
        }
        None
    }

    pub(super) async fn preferred_watcher_notification_channel_hint(
        &self,
        watcher: &super::watcher::Watcher,
    ) -> Option<String> {
        if normalize_automation_notification_channel(Some(&watcher.notify_channel)) != "preferred" {
            return None;
        }
        let session_id = super::background_session::background_session_id_from_automation(
            &watcher.poll_arguments,
        );
        if let Some(session_id) = session_id.as_deref() {
            if let Some(session) = self.background_sessions.get(session_id).await {
                if let Some(preferred) = session.preferred_delivery_channel.as_deref() {
                    let normalized = preferred.trim().to_ascii_lowercase();
                    if !normalized.is_empty()
                        && (!is_external_notification_channel(&normalized)
                            || self
                                .notification_channel_is_configured_any(&normalized)
                                .await)
                    {
                        return Some(normalized);
                    }
                }
            }
        }
        None
    }

    pub(super) fn webhook_task_metadata(task: &super::task::Task) -> Option<&serde_json::Value> {
        let automation = task.arguments.get("_automation")?;
        if automation.get("kind").and_then(|value| value.as_str()) != Some("webhook") {
            return None;
        }
        automation.get("webhook")
    }

    pub(super) fn webhook_meta_string(
        meta: Option<&serde_json::Value>,
        key: &str,
        fallback: Option<&str>,
    ) -> Option<String> {
        meta.and_then(|value| value.get(key))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .or_else(|| {
                fallback
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
            })
    }

    pub(super) fn webhook_meta_bool(
        meta: Option<&serde_json::Value>,
        key: &str,
        fallback: bool,
    ) -> bool {
        meta.and_then(|value| value.get(key))
            .and_then(|value| value.as_bool())
            .unwrap_or(fallback)
    }

    pub(super) fn task_completion_plugin_payload(
        event_name: &str,
        task: &super::task::Task,
        output: Option<&str>,
        failure: Option<&str>,
    ) -> serde_json::Value {
        serde_json::json!({
            "event": event_name,
            "task": {
                "id": task.id.to_string(),
                "description": task.description.clone(),
                "action": task.action.clone(),
                "status": if failure.is_some() { "failed" } else { "completed" },
                "approval": match super::task::normalized_task_approval(&task.approval) {
                    super::task::TaskApproval::Auto => "auto",
                    super::task::TaskApproval::RequireApproval => "require_approval",
                    super::task::TaskApproval::NotifyThenExecute { .. } => "notify_then_execute",
                },
                "created_at": task.created_at.to_rfc3339(),
                "scheduled_for": task.scheduled_for.as_ref().map(|value| value.to_rfc3339()),
                "report_to": Self::task_report_target(task),
                "is_webhook_task": Self::webhook_task_metadata(task).is_some(),
                "capabilities": task.capabilities.clone(),
            },
            "result": output
                .map(Self::delivery_output_text)
                .map(|text| automation_truncate_text(&text, 600)),
            "failure": failure
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| automation_truncate_text(value, 600)),
            "webhook": Self::webhook_task_metadata(task).cloned(),
        })
    }

    pub(super) fn approval_plugin_payload(
        task: &super::task::Task,
        metadata: &ApprovalRequestMetadata,
    ) -> serde_json::Value {
        serde_json::json!({
            "event": "approval.requested",
            "task": {
                "id": task.id.to_string(),
                "description": task.description.clone(),
                "action": task.action.clone(),
                "status": "awaiting_approval",
                "created_at": task.created_at.to_rfc3339(),
                "report_to": Self::task_report_target(task),
                "is_webhook_task": Self::webhook_task_metadata(task).is_some(),
            },
            "approval": {
                "title": metadata.title,
                "summary": metadata.summary,
                "reason": metadata.reason,
                "rule_name": metadata.rule_name,
                "risk_level": metadata.risk_level,
                "risk_score": metadata.risk_score,
                "source": metadata.source,
            }
        })
    }

    pub(super) fn delivery_output_text(output: &str) -> String {
        let trimmed = output.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            for key in ["response", "brief", "summary", "message"] {
                if let Some(text) = value
                    .get(key)
                    .and_then(|entry| entry.as_str())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    return text.to_string();
                }
            }
            if let Some(result_text) = value
                .get("result")
                .and_then(|entry| entry.as_str())
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                return result_text.to_string();
            }
        }
        trimmed.to_string()
    }

    pub(super) fn webhook_delivery_message(
        task: &super::task::Task,
        output: Option<&str>,
        failure: Option<&str>,
    ) -> String {
        let meta = Self::webhook_task_metadata(task);
        let source_name = Self::webhook_meta_string(
            meta,
            "source_name",
            task.arguments
                .get("source_name")
                .and_then(|value| value.as_str()),
        )
        .unwrap_or_else(|| "Webhook".to_string());
        let event_type = Self::webhook_meta_string(
            meta,
            "event_type",
            task.arguments
                .get("event_type")
                .and_then(|value| value.as_str()),
        )
        .unwrap_or_else(|| "webhook".to_string());
        let subject = Self::webhook_meta_string(
            meta,
            "subject",
            task.arguments
                .get("subject")
                .and_then(|value| value.as_str()),
        )
        .unwrap_or_else(|| task.description.clone());
        let event_status = Self::webhook_meta_string(
            meta,
            "event_status",
            task.arguments
                .get("event_status")
                .and_then(|value| value.as_str()),
        );
        let event_url = Self::webhook_meta_string(
            meta,
            "event_url",
            task.arguments
                .get("event_url")
                .and_then(|value| value.as_str()),
        );
        let mut lines = vec![if failure.is_some() {
            format!("Webhook failed: {}", source_name)
        } else {
            format!("Webhook completed: {}", source_name)
        }];
        lines.push(format!("Event: {}", event_type));
        lines.push(format!("Subject: {}", subject));
        if let Some(status) = event_status {
            lines.push(format!("Status: {}", status));
        }
        lines.push("Saved in Tasks.".to_string());
        if let Some(url) = event_url {
            lines.push(format!("Reference: {}", url));
        }
        if let Some(text) = output
            .map(Self::delivery_output_text)
            .filter(|text| !text.is_empty())
        {
            lines.push(String::new());
            lines.push("Result:".to_string());
            lines.push(safe_truncate(&text, 1200));
        }
        if let Some(error) = failure.map(str::trim).filter(|text| !text.is_empty()) {
            lines.push(String::new());
            lines.push("Failure:".to_string());
            lines.push(safe_truncate(error, 1200));
        }
        lines.join("\n")
    }

    pub(super) async fn deliver_task_report_message(
        &self,
        task: &super::task::Task,
        message: &str,
    ) {
        if task.action == "daily_brief" {
            tracing::debug!(
                "Skipping task report notification for scheduled daily_brief generation"
            );
            return;
        }
        if message.trim().is_empty() {
            return;
        }
        if Self::task_report_is_chat_owned(task) {
            tracing::debug!(
                "Skipping task report notification for chat-owned task '{}'",
                task.description
            );
            return;
        }
        let report_to = Self::task_report_target(task);
        let preferred_hint = self.preferred_task_report_channel_hint(task).await;
        let is_webhook = Self::webhook_task_metadata(task).is_some();
        if report_to.is_empty() || report_to == AUTOMATION_IN_APP_NOTIFICATION_CHANNEL {
            if !is_webhook {
                self.emit_notification("Scheduled Task Result", message, "info", "scheduler")
                    .await;
            }
            return;
        }
        if report_to == "preferred" {
            if !is_webhook {
                self.emit_notification("Scheduled Task Result", message, "info", "scheduler")
                    .await;
            }
            let delivered = self
                .notify_preferred_channel_reported_with_hint(
                    message,
                    preferred_hint.as_deref(),
                    !super::task::task_is_scheduled_reminder(task),
                )
                .await
                .into_iter()
                .any(|outcome| outcome.success);
            if !delivered {
                tracing::info!(
                    "Task '{}' completed with in-app notification only (no preferred notification channel available)",
                    task.description
                );
            }
            return;
        }
        if is_external_notification_channel(&report_to) {
            let outcome = if self
                .notification_channel_is_configured_any(&report_to)
                .await
            {
                self.try_send_notification_reported(&report_to, message)
                    .await
            } else {
                notification_channel_not_connected_outcome(&report_to)
            };
            if outcome.success && is_external_notification_channel(&outcome.channel) {
                return;
            }
            let delivery_label = notification_channel_display_name(&report_to);
            let error = outcome
                .error
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let body = if let Some(error) = error {
                format!(
                    "Task '{}' completed. {} delivery is unavailable, so the result stayed in-app.\n\nDelivery error: {}\n\n{}",
                    task.description, delivery_label, error, message
                )
            } else {
                format!(
                    "Task '{}' completed. {} delivery is unavailable, so the result stayed in-app.\n\n{}",
                    task.description, delivery_label, message
                )
            };
            if is_webhook {
                self.emit_notification_forced("Delivery Setup Needed", &body, "warning", "webhook")
                    .await;
            } else {
                self.emit_notification("Scheduled Task Result", &body, "info", "scheduler")
                    .await;
            }
            return;
        }
        tracing::info!(
            "Sending automated task result to channel={} (task={})",
            report_to,
            task.description
        );
        let delivery_outcome = self
            .try_send_notification_reported(&report_to, message)
            .await;
        if !delivery_outcome.success {
            tracing::warn!(
                "Task '{}' completed but delivery to '{}' failed: {:?}",
                task.description,
                report_to,
                delivery_outcome.error
            );
            let failure_title = if is_webhook {
                "Webhook delivery failed"
            } else {
                "Scheduled Task Delivery Failed"
            };
            let failure_body = if let Some(error) = delivery_outcome
                .error
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                format!(
                    "Task '{}' completed, but delivery to '{}' failed.\n\nDelivery error: {}\n\n{}",
                    task.description, report_to, error, message
                )
            } else {
                format!(
                    "Task '{}' completed, but delivery to '{}' failed.\n\n{}",
                    task.description, report_to, message
                )
            };
            if is_webhook {
                self.emit_notification_forced(failure_title, &failure_body, "warning", "webhook")
                    .await;
            } else {
                self.emit_notification(failure_title, &failure_body, "warning", "scheduler")
                    .await;
            }
        }
    }

    pub(super) async fn deliver_task_output(&self, task: &super::task::Task, output: &str) {
        let message = if Self::webhook_task_metadata(task).is_some() {
            Self::webhook_delivery_message(task, Some(output), None)
        } else {
            Self::delivery_output_text(output)
        };
        self.deliver_task_report_message(task, &message).await;
    }

    pub(super) async fn maybe_emit_webhook_task_completion_notification(
        &self,
        task: &super::task::Task,
        output: Option<&str>,
        failure: Option<&str>,
    ) {
        let meta = Self::webhook_task_metadata(task);
        if meta.is_none() {
            return;
        }
        let is_success = failure.is_none();
        let should_notify = if is_success {
            Self::webhook_meta_bool(meta, "notify_on_success", true)
        } else {
            Self::webhook_meta_bool(meta, "notify_on_failure", true)
        };
        let source_name = Self::webhook_meta_string(meta, "source_name", None)
            .unwrap_or_else(|| "Webhook".to_string());
        let body = Self::webhook_delivery_message(task, output, failure);
        if should_notify {
            let title = if is_success {
                format!("Webhook completed: {}", source_name)
            } else {
                format!("Webhook failed: {}", source_name)
            };
            let level = if is_success { "info" } else { "error" };
            self.emit_notification_forced(&title, &body, level, "webhook")
                .await;
        }
        if !is_success && !Self::task_report_target(task).is_empty() {
            self.deliver_task_report_message(task, &body).await;
        }
    }

    pub async fn dispatch_plugin_event(
        &self,
        event_name: &str,
        payload: serde_json::Value,
    ) -> Result<()> {
        self.plugins
            .write()
            .await
            .dispatch_event(event_name, &payload)
            .await
    }

    pub async fn execute_task_supervised_shared(
        agent: &std::sync::Arc<tokio::sync::RwLock<Self>>,
        task: super::task::Task,
    ) {
        let storage = { agent.read().await.storage.clone() };
        let policy = automation_policy_from_arguments(
            &task.arguments,
            default_automation_validation_for_action(&task.action),
        );
        let origin = automation_origin_from_arguments(&task.arguments);
        let attempt = automation_current_attempt(&task.arguments);
        let started_at = chrono::Utc::now();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut run_record_available = false;
        let running_run_record = task_automation_run_record(
            &task,
            &run_id,
            AutomationRunStatus::Running,
            attempt,
            started_at,
            None,
            origin.clone(),
            policy.clone(),
            AutomationCritique {
                summary: "Execution started.".to_string(),
                retryable: false,
                validation_passed: false,
            },
            None,
            None,
            None,
        );
        if let Err(error) = append_automation_run(&storage, running_run_record).await {
            tracing::warn!(
                "Failed to append running automation run record for task '{}': {}",
                task.id,
                error
            );
        } else {
            run_record_available = true;
        }

        let mut supervisor_state = load_automation_supervisor_state(&storage, &task.id.to_string())
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| AutomationSupervisorState {
                automation_id: task.id.to_string(),
                automation_kind: "task".to_string(),
                title: task.description.clone(),
                action: task.action.clone(),
                status: "queued".to_string(),
                attempt_count: 0,
                consecutive_failures: 0,
                last_run_id: None,
                last_run_at: None,
                last_success_at: None,
                last_error: None,
                next_retry_at: None,
                stalled_count: 0,
                origin: origin.clone(),
                created_at: Some(task.created_at.to_rfc3339()),
            });
        supervisor_state.status = "running".to_string();
        supervisor_state.attempt_count = attempt;
        supervisor_state.last_run_id = run_record_available.then(|| run_id.clone());
        supervisor_state.last_run_at = Some(started_at.to_rfc3339());
        supervisor_state.next_retry_at = None;
        supervisor_state.origin = origin.clone();
        if supervisor_state.created_at.is_none() {
            supervisor_state.created_at = Some(task.created_at.to_rfc3339());
        }
        if let Err(error) =
            upsert_automation_supervisor_state(&storage, supervisor_state.clone()).await
        {
            tracing::warn!(
                "Failed to persist running supervisor state for task '{}': {}",
                task.id,
                error
            );
        }
        let running_last_run_id = if run_record_available {
            Some(run_id.as_str())
        } else {
            None
        };
        if let Err(error) = storage
            .record_task_run_metadata(
                &task.id.to_string(),
                running_last_run_id,
                None,
                Some(supervisor_state.consecutive_failures as i32),
            )
            .await
        {
            tracing::warn!(
                "Failed to persist task run metadata for running task '{}': {}",
                task.id,
                error
            );
        }

        let task_agent = Agent::snapshot(agent).await;
        let execution = if policy.stall_timeout_secs > 0 {
            tokio::time::timeout(
                std::time::Duration::from_secs(policy.stall_timeout_secs),
                task_agent.execute_task(&task),
            )
            .await
            .ok()
        } else {
            Some(task_agent.execute_task(&task).await)
        };
        let finished_at = chrono::Utc::now();
        tracing::info!(
            "Automation supervisor: task '{}' attempt {}/{} finished",
            task.description,
            attempt,
            policy.max_attempts
        );

        let (run_status, mut task_status, output, error_text) = match execution {
            Some(Ok(output)) => {
                let critique = critique_automation_result(&policy.validation, Some(&output), None);
                if !critique.validation_passed {
                    (
                        AutomationRunStatus::Failed,
                        super::task::TaskStatus::Failed {
                            error: critique.summary.clone(),
                        },
                        Some(output),
                        Some(critique.summary),
                    )
                } else {
                    (
                        AutomationRunStatus::Succeeded,
                        super::task::TaskStatus::Completed,
                        Some(output),
                        None,
                    )
                }
            }
            Some(Err(error)) => {
                let error_text = error.to_string();
                (
                    AutomationRunStatus::Failed,
                    super::task::TaskStatus::Failed {
                        error: error_text.clone(),
                    },
                    None,
                    Some(error_text),
                )
            }
            None => {
                let error_text = format!(
                    "Background execution timed out after {} seconds",
                    policy.stall_timeout_secs
                );
                (
                    AutomationRunStatus::TimedOut,
                    super::task::TaskStatus::Failed {
                        error: error_text.clone(),
                    },
                    None,
                    Some(error_text),
                )
            }
        };
        let input_needed_result = output
            .as_deref()
            .and_then(Self::parse_scheduled_input_needed_result);
        if input_needed_result.is_some() {
            task_status = super::task::TaskStatus::Paused;
        }

        let critique = critique_automation_result(
            &policy.validation,
            output.as_deref(),
            error_text.as_deref(),
        );
        let should_retry = matches!(
            run_status,
            AutomationRunStatus::Failed | AutomationRunStatus::TimedOut
        ) && critique.retryable
            && input_needed_result.is_none()
            && attempt < policy.max_attempts
            && should_retry_background_action(&task.action);
        let next_retry_at = should_retry.then(|| compute_retry_at(finished_at, &policy, attempt));
        let stored_input_needed_result = input_needed_result
            .as_ref()
            .and_then(|value| serde_json::to_string(value).ok());
        let output_preview = if let Some(input_needed) = input_needed_result.as_ref() {
            Some(automation_truncate_text(
                &input_needed.notification_body,
                260,
            ))
        } else {
            output
                .as_deref()
                .map(|value| automation_truncate_text(value, 260))
        };
        let mut effective_run_status = run_status.clone();
        if input_needed_result.is_some() {
            effective_run_status = AutomationRunStatus::Triggered;
        }
        if should_retry {
            effective_run_status = AutomationRunStatus::Retrying;
        }

        let run_record = task_automation_run_record(
            &task,
            &run_id,
            effective_run_status.clone(),
            attempt,
            started_at,
            Some(finished_at),
            origin.clone(),
            policy.clone(),
            critique.clone(),
            output_preview,
            error_text.clone(),
            next_retry_at.clone(),
        );
        if let Err(error) = append_automation_run(&storage, run_record).await {
            tracing::warn!(
                "Failed to append automation run record for task '{}': {}",
                task.id,
                error
            );
        } else {
            run_record_available = true;
        }

        supervisor_state.last_run_at = Some(finished_at.to_rfc3339());
        supervisor_state.last_run_id = run_record_available.then(|| run_id.clone());
        supervisor_state.origin = origin;
        match effective_run_status {
            AutomationRunStatus::Succeeded => {
                supervisor_state.status = "succeeded".to_string();
                supervisor_state.consecutive_failures = 0;
                supervisor_state.last_error = None;
                supervisor_state.last_success_at = Some(finished_at.to_rfc3339());
                supervisor_state.next_retry_at = None;
                if let Err(error) = storage
                    .record_task_run_metadata(
                        &task.id.to_string(),
                        supervisor_state.last_run_id.as_deref(),
                        None,
                        Some(0),
                    )
                    .await
                {
                    tracing::warn!(
                        "Failed to persist success metadata for task '{}': {}",
                        task.id,
                        error
                    );
                }
                if let Some(ref value) = output {
                    if let Err(error) = task_agent
                        .finalize_task(
                            task.id,
                            super::task::TaskStatus::Completed,
                            Some(value.clone()),
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to finalize completed task '{}': {}",
                            task.id,
                            error
                        );
                    }
                    if scheduled_task_should_deliver_output_after_execution(&task) {
                        task_agent.deliver_task_output(&task, value).await;
                    }
                    task_agent
                        .maybe_emit_webhook_task_completion_notification(&task, Some(value), None)
                        .await;
                    if let Err(error) = task_agent
                        .dispatch_plugin_event(
                            "task.completed",
                            Self::task_completion_plugin_payload(
                                "task.completed",
                                &task,
                                Some(value),
                                None,
                            ),
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to dispatch plugin event task.completed for task '{}': {}",
                            task.id,
                            error
                        );
                    }
                } else if let Err(error) = {
                    task_agent
                        .finalize_task(task.id, super::task::TaskStatus::Completed, None)
                        .await
                } {
                    tracing::warn!("Failed to finalize completed task '{}': {}", task.id, error);
                } else {
                    task_agent
                        .maybe_emit_webhook_task_completion_notification(&task, None, None)
                        .await;
                    if let Err(error) = task_agent
                        .dispatch_plugin_event(
                            "task.completed",
                            Self::task_completion_plugin_payload(
                                "task.completed",
                                &task,
                                None,
                                None,
                            ),
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to dispatch plugin event task.completed for task '{}': {}",
                            task.id,
                            error
                        );
                    }
                }
            }
            AutomationRunStatus::Retrying => {
                let retry_at = next_retry_at
                    .unwrap_or_else(|| compute_retry_at(finished_at, &policy, attempt));
                supervisor_state.status = "retrying".to_string();
                supervisor_state.consecutive_failures =
                    supervisor_state.consecutive_failures.saturating_add(1u32);
                supervisor_state.last_error = Some(critique.summary.clone());
                supervisor_state.next_retry_at = Some(retry_at.to_rfc3339());
                if let Err(error) = storage
                    .record_task_run_metadata(
                        &task.id.to_string(),
                        supervisor_state.last_run_id.as_deref(),
                        supervisor_state.next_retry_at.as_deref(),
                        Some(supervisor_state.consecutive_failures as i32),
                    )
                    .await
                {
                    tracing::warn!(
                        "Failed to persist retry metadata for task '{}': {}",
                        task.id,
                        error
                    );
                }
                let summary = output
                    .as_deref()
                    .map(|value| automation_truncate_text(value, 240))
                    .or_else(|| error_text.clone())
                    .unwrap_or_else(|| critique.summary.clone());
                if let Err(error) = task_agent
                    .reschedule_task_retry(&task, retry_at, &summary, attempt)
                    .await
                {
                    tracing::warn!(
                        "Failed to reschedule retry for task '{}': {}",
                        task.id,
                        error
                    );
                }
            }
            AutomationRunStatus::Failed
            | AutomationRunStatus::TimedOut
            | AutomationRunStatus::Triggered => {
                if let Some(_input_needed) = input_needed_result.as_ref() {
                    tracing::info!(
                        "Automation supervisor: task '{}' paused for missing inputs",
                        task.description
                    );
                    supervisor_state.status = "paused".to_string();
                    supervisor_state.consecutive_failures = 0;
                    supervisor_state.last_error = None;
                    supervisor_state.next_retry_at = None;
                    if let Err(error) = storage
                        .record_task_run_metadata(
                            &task.id.to_string(),
                            supervisor_state.last_run_id.as_deref(),
                            None,
                            Some(0),
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist paused metadata for task '{}': {}",
                            task.id,
                            error
                        );
                    }
                    if let Err(error) = task_agent
                        .finalize_task(
                            task.id,
                            task_status.clone(),
                            stored_input_needed_result
                                .clone()
                                .or_else(|| output.clone()),
                        )
                        .await
                    {
                        tracing::warn!("Failed to finalize paused task '{}': {}", task.id, error);
                    }
                } else {
                    supervisor_state.status = match effective_run_status {
                        AutomationRunStatus::TimedOut => "timed_out".to_string(),
                        _ => "failed".to_string(),
                    };
                    supervisor_state.consecutive_failures =
                        supervisor_state.consecutive_failures.saturating_add(1u32);
                    supervisor_state.last_error = Some(critique.summary.clone());
                    supervisor_state.next_retry_at = None;
                    if let Err(error) = storage
                        .record_task_run_metadata(
                            &task.id.to_string(),
                            supervisor_state.last_run_id.as_deref(),
                            None,
                            Some(supervisor_state.consecutive_failures as i32),
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist failure metadata for task '{}': {}",
                            task.id,
                            error
                        );
                    }
                    let final_result = output.clone().or_else(|| error_text.clone());
                    if let Err(error) = task_agent
                        .finalize_task(task.id, task_status, final_result)
                        .await
                    {
                        tracing::warn!("Failed to finalize failed task '{}': {}", task.id, error);
                    }
                    task_agent
                        .maybe_emit_webhook_task_completion_notification(
                            &task,
                            output.as_deref(),
                            Some(&critique.summary),
                        )
                        .await;
                    if let Err(error) = task_agent
                        .dispatch_plugin_event(
                            "task.failed",
                            Self::task_completion_plugin_payload(
                                "task.failed",
                                &task,
                                output.as_deref(),
                                Some(&critique.summary),
                            ),
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to dispatch plugin event task.failed for task '{}': {}",
                            task.id,
                            error
                        );
                    }
                }
            }
            AutomationRunStatus::Running => {}
        }
        if let Err(error) = upsert_automation_supervisor_state(&storage, supervisor_state).await {
            tracing::warn!(
                "Failed to persist final supervisor state for task '{}': {}",
                task.id,
                error
            );
        }
    }

    pub(super) async fn first_watcher_notification_image_from_data_dir(
        data_dir: &std::path::Path,
        result: &str,
    ) -> Option<WatcherNotificationImage> {
        for web_path in watcher_result_output_files(result) {
            if !watcher_output_file_is_image(&web_path) {
                continue;
            }
            let Some((exec_id, filename)) = parse_output_web_path(&web_path) else {
                continue;
            };
            let file_path = data_dir.join("outputs").join(&exec_id).join(&filename);
            let Ok(bytes) = tokio::fs::read(&file_path).await else {
                continue;
            };
            if bytes.is_empty() {
                continue;
            }
            return Some(WatcherNotificationImage {
                web_path,
                filename,
                bytes,
            });
        }
        None
    }

    pub(super) async fn try_send_notification_image_reported(
        &self,
        channel: &str,
        caption: &str,
        image: &WatcherNotificationImage,
    ) -> NotificationDispatchOutcome {
        let channel_name = format!("{}:image", channel);
        if !matches!(channel, "telegram" | "whatsapp") {
            return NotificationDispatchOutcome::pre_send_failure(
                channel_name,
                "Channel does not support image notification attachments yet",
            );
        }
        let mut safe_caption = match Self::sanitize_outbound_notification_message(channel, caption)
        {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!("{}", error);
                return NotificationDispatchOutcome::pre_send_failure(channel_name, error);
            }
        };
        if safe_caption.chars().count() > 950 {
            safe_caption = format!(
                "{}\n\nSnapshot: {}",
                safe_caption.chars().take(900).collect::<String>(),
                image.filename
            );
        }
        let image_url = format!(
            "{}{}",
            crate::core::net::internal_api_base_url().trim_end_matches('/'),
            image.web_path
        );
        match crate::channels::send_screenshot(
            self,
            channel,
            &image.bytes,
            &safe_caption,
            Some(&image_url),
        )
        .await
        {
            Ok(()) => NotificationDispatchOutcome::full_success(channel_name),
            Err(error) => {
                NotificationDispatchOutcome::pre_send_failure(channel_name, error.to_string())
            }
        }
    }

    pub(crate) async fn handle_watcher_trigger_supervised(
        &self,
        watcher: super::watcher::Watcher,
        result: String,
        prepared: WatcherFollowupPreparation,
    ) {
        let WatcherFollowupPreparation {
            origin,
            policy,
            attempt,
            started_at,
            finished_at,
            notification_image,
            output,
            suppress_external_reason,
        } = prepared;
        let run_id = uuid::Uuid::new_v4().to_string();
        let output = Some(output);
        let error_text: Option<String> = None;
        let status = AutomationRunStatus::Triggered;
        let critique = critique_automation_result(
            &policy.validation,
            output.as_deref(),
            error_text.as_deref(),
        );
        let notify_text = match (&output, &error_text) {
            (Some(response), _) => response.clone(),
            (_, Some(error)) => format!(
                "A watcher matched, but the follow-up summary failed: {}\n\nRaw result:\n{}",
                error, result
            ),
            _ => result.clone(),
        };
        let external_suppression_reason = suppress_external_reason.or_else(|| {
            (!watcher_notification_text_is_useful(&notify_text))
                .then(|| "notification text was empty, raw, or error-like".to_string())
        });
        let in_app_notify_text = external_suppression_reason
            .as_deref()
            .map(|reason| watcher_internal_match_text(&watcher, &result, reason))
            .unwrap_or_else(|| notify_text.clone());

        let run_record = AutomationRunRecord {
            id: run_id.clone(),
            automation_id: watcher.id.to_string(),
            automation_kind: "watcher".to_string(),
            title: watcher.description.clone(),
            action: watcher.poll_action.clone(),
            trigger: "watcher".to_string(),
            status,
            attempt,
            started_at: started_at.to_rfc3339(),
            completed_at: Some(finished_at.to_rfc3339()),
            duration_ms: Some(
                finished_at
                    .signed_duration_since(started_at)
                    .num_milliseconds()
                    .max(0) as u64,
            ),
            origin: origin.clone(),
            policy,
            critique: critique.clone(),
            output_preview: Some(automation_truncate_text(&in_app_notify_text, 260)),
            error: error_text.clone(),
            next_retry_at: None,
        };
        if let Err(error) = append_automation_run(&self.storage, run_record).await {
            tracing::warn!(
                "Failed to append watcher automation run record for '{}': {}",
                watcher.id,
                error
            );
        }

        let existing_state =
            load_automation_supervisor_state(&self.storage, &watcher.id.to_string())
                .await
                .ok()
                .flatten();
        let state = AutomationSupervisorState {
            automation_id: watcher.id.to_string(),
            automation_kind: "watcher".to_string(),
            title: watcher.description.clone(),
            action: watcher.poll_action.clone(),
            status: if error_text.is_some() {
                "failed".to_string()
            } else if watcher.repeat_on_match {
                "active".to_string()
            } else {
                "triggered".to_string()
            },
            attempt_count: watcher.poll_count.saturating_add(1).max(attempt),
            consecutive_failures: if error_text.is_some() { 1 } else { 0 },
            last_run_id: Some(run_id),
            last_run_at: Some(finished_at.to_rfc3339()),
            last_success_at: if error_text.is_none() {
                Some(finished_at.to_rfc3339())
            } else {
                None
            },
            last_error: error_text.clone(),
            next_retry_at: None,
            stalled_count: 0,
            origin,
            created_at: existing_state
                .as_ref()
                .and_then(|state| state.created_at.clone())
                .or_else(|| Some(watcher.created_at.to_rfc3339())),
        };
        if let Err(error) = upsert_automation_supervisor_state(&self.storage, state).await {
            tracing::warn!(
                "Failed to persist watcher supervisor state for '{}': {}",
                watcher.id,
                error
            );
        }

        self.emit_notification("Watcher Triggered", &in_app_notify_text, "info", "watcher")
            .await;
        if let Some(reason) = external_suppression_reason {
            self.watcher_manager
                .push_notification_attempt(
                    watcher.id,
                    super::watcher::WatcherNotificationAttempt {
                        attempted_at: chrono::Utc::now(),
                        channel: "external".to_string(),
                        success: false,
                        message: in_app_notify_text,
                        error: Some(format!("Notification suppressed: {}", reason)),
                    },
                )
                .await;
            return;
        }
        let requested_channel =
            normalize_automation_notification_channel(Some(&watcher.notify_channel));
        let preferred_hint = self
            .preferred_watcher_notification_channel_hint(&watcher)
            .await;
        let authorization_delivery_channel = if requested_channel == "preferred" {
            preferred_hint
                .clone()
                .unwrap_or_else(|| "preferred".to_string())
        } else {
            requested_channel.clone()
        };
        if requested_channel == "preferred"
            || is_external_notification_channel(&authorization_delivery_channel)
        {
            let notification_auth = automation_runtime_authorization_context(
                &watcher.poll_arguments,
                crate::actions::ActionExecutionSurface::Background,
            );
            let notify_arguments = serde_json::json!({
                "message": notify_text.clone(),
                "delivery_channel": authorization_delivery_channel.clone(),
            });
            let action_def = self.runtime.action_definition("notify_user").await;
            let decision = self
                .runtime
                .authorize_action_invocation(
                    "notify_user",
                    action_def.as_ref(),
                    &notify_arguments,
                    &notification_auth,
                )
                .await;
            match decision {
                Ok(decision) if decision.allowed => {}
                Ok(decision) => {
                    self.watcher_manager
                        .push_notification_attempt(
                            watcher.id,
                            super::watcher::WatcherNotificationAttempt {
                                attempted_at: chrono::Utc::now(),
                                channel: authorization_delivery_channel.clone(),
                                success: false,
                                message: notify_text.clone(),
                                error: Some(decision.reason),
                            },
                        )
                        .await;
                    return;
                }
                Err(error) => {
                    self.watcher_manager
                        .push_notification_attempt(
                            watcher.id,
                            super::watcher::WatcherNotificationAttempt {
                                attempted_at: chrono::Utc::now(),
                                channel: authorization_delivery_channel.clone(),
                                success: false,
                                message: notify_text.clone(),
                                error: Some(format!(
                                    "Notification authorization failed: {}",
                                    error
                                )),
                            },
                        )
                        .await;
                    return;
                }
            }
        }
        if requested_channel == "preferred" {
            let outcomes = self
                .notify_preferred_channel_reported_with_hint(
                    &notify_text,
                    preferred_hint.as_deref(),
                    true,
                )
                .await;
            for outcome in outcomes {
                if outcome.success && is_external_notification_channel(&outcome.channel) {
                    if let Some(image) = notification_image.as_ref() {
                        let image_outcome = self
                            .try_send_notification_image_reported(
                                &outcome.channel,
                                &notify_text,
                                image,
                            )
                            .await;
                        if !image_outcome.success {
                            tracing::warn!(
                                "Watcher '{}' image notification failed via '{}': {:?}",
                                watcher.id,
                                image_outcome.channel,
                                image_outcome.error
                            );
                        }
                    }
                }
                self.watcher_manager
                    .push_notification_attempt(
                        watcher.id,
                        super::watcher::WatcherNotificationAttempt {
                            attempted_at: chrono::Utc::now(),
                            channel: outcome.channel,
                            success: outcome.success,
                            message: notify_text.clone(),
                            error: outcome.error,
                        },
                    )
                    .await;
            }
        } else if requested_channel.is_empty()
            || requested_channel == AUTOMATION_IN_APP_NOTIFICATION_CHANNEL
        {
            self.watcher_manager
                .push_notification_attempt(
                    watcher.id,
                    super::watcher::WatcherNotificationAttempt {
                        attempted_at: chrono::Utc::now(),
                        channel: "web".to_string(),
                        success: true,
                        message: in_app_notify_text,
                        error: None,
                    },
                )
                .await;
        } else if !requested_channel.is_empty() {
            if is_external_notification_channel(&requested_channel) {
                let outcome = if self
                    .notification_channel_is_configured_any(&requested_channel)
                    .await
                {
                    self.try_send_notification_reported(&requested_channel, &notify_text)
                        .await
                } else {
                    notification_channel_not_connected_outcome(&requested_channel)
                };
                if outcome.success && is_external_notification_channel(&outcome.channel) {
                    if let Some(image) = notification_image.as_ref() {
                        let image_outcome = self
                            .try_send_notification_image_reported(
                                &outcome.channel,
                                &notify_text,
                                image,
                            )
                            .await;
                        if !image_outcome.success {
                            tracing::warn!(
                                "Watcher '{}' image notification failed via '{}': {:?}",
                                watcher.id,
                                image_outcome.channel,
                                image_outcome.error
                            );
                        }
                    }
                }
                self.watcher_manager
                    .push_notification_attempt(
                        watcher.id,
                        super::watcher::WatcherNotificationAttempt {
                            attempted_at: chrono::Utc::now(),
                            channel: outcome.channel,
                            success: outcome.success,
                            message: notify_text.clone(),
                            error: outcome.error,
                        },
                    )
                    .await;
                return;
            }
            let outcome = self
                .try_send_notification_reported(&requested_channel, &notify_text)
                .await;
            if outcome.success {
                if let Some(image) = notification_image.as_ref() {
                    let image_outcome = self
                        .try_send_notification_image_reported(
                            &requested_channel,
                            &notify_text,
                            image,
                        )
                        .await;
                    if !image_outcome.success {
                        tracing::warn!(
                            "Watcher '{}' image notification failed via '{}': {:?}",
                            watcher.id,
                            image_outcome.channel,
                            image_outcome.error
                        );
                    }
                }
            }
            let channel = outcome.channel;
            let success = outcome.success;
            let error = outcome.error;
            self.watcher_manager
                .push_notification_attempt(
                    watcher.id,
                    super::watcher::WatcherNotificationAttempt {
                        attempted_at: chrono::Utc::now(),
                        channel,
                        success,
                        message: notify_text.clone(),
                        error,
                    },
                )
                .await;
        }
    }

    pub(super) async fn build_goal_progress_report(&self, goal_id: Option<&str>) -> Result<String> {
        let tasks = self.tasks.read().await;
        let goal_tasks: Vec<&super::task::Task> = tasks
            .all()
            .iter()
            .filter(|t| t.action == "goal")
            .filter(|t| {
                if let Some(gid) = goal_id {
                    t.id.to_string() == gid
                } else {
                    true
                }
            })
            .collect();

        let mut related: Vec<&super::task::Task> = tasks
            .all()
            .iter()
            .filter(|t| {
                if let Some(gid) = goal_id {
                    t.arguments.get("goal_id").and_then(|v| v.as_str()) == Some(gid)
                } else {
                    t.arguments.get("goal_id").is_some()
                }
            })
            .collect();

        if goal_id.is_none() {
            related = tasks
                .all()
                .iter()
                .filter(|t| t.action != "goal_progress_report" && t.action != "daily_brief")
                .take(20)
                .collect();
        }

        let total = related.len();
        let completed = related
            .iter()
            .filter(|t| matches!(t.status, super::task::TaskStatus::Completed))
            .count();
        let pending = related
            .iter()
            .filter(|t| {
                matches!(
                    t.status,
                    super::task::TaskStatus::Pending
                        | super::task::TaskStatus::AwaitingApproval
                        | super::task::TaskStatus::InProgress
                )
            })
            .count();

        let goals_text = if goal_tasks.is_empty() {
            "No explicit goal record found.".to_string()
        } else {
            goal_tasks
                .iter()
                .map(|g| format!("- {}", g.description))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let pending_text = related
            .iter()
            .filter(|t| {
                matches!(
                    t.status,
                    super::task::TaskStatus::Pending
                        | super::task::TaskStatus::AwaitingApproval
                        | super::task::TaskStatus::InProgress
                )
            })
            .take(5)
            .map(|t| format!("- {} ({:?})", t.description, t.status))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "Generate a concise goal progress report.\n\
Goal reference:\n{}\n\n\
Metrics: total_related_tasks={}, completed={}, pending_or_running={}\n\n\
Top pending:\n{}\n\n\
Return: 1 short status paragraph + 3 bullet next steps.",
            goals_text,
            total,
            completed,
            pending,
            if pending_text.is_empty() {
                "None"
            } else {
                &pending_text
            }
        );

        let empty_actions: Vec<crate::actions::ActionDef> = Vec::new();
        match self
            .supervised_internal_chat(
                "goal_progress",
                "goal_progress_report",
                "goal_progress_report",
                &ModelRole::Primary,
                vec![],
                "You are a pragmatic execution coach. Be concise and actionable.",
                &prompt,
                &[],
                &empty_actions,
                internal_llm_timeout_ms("AGENTARK_GOAL_PROGRESS_REPORT_TIMEOUT_MS", 20_000),
                2,
            )
            .await
        {
            Some(resp) => Ok(resp.content),
            None => Ok(format!(
                "Goal progress: {} of {} related tasks completed. {} still active.",
                completed, total, pending
            )),
        }
    }

    pub(super) fn required_action_argument_present(value: Option<&serde_json::Value>) -> bool {
        match value {
            Some(serde_json::Value::Null) | None => false,
            Some(serde_json::Value::String(s)) => !s.trim().is_empty(),
            Some(serde_json::Value::Array(items)) => !items.is_empty(),
            Some(serde_json::Value::Object(map)) => !map.is_empty(),
            Some(_) => true,
        }
    }

    pub(super) fn infer_code_execute_language_from_code(code: &str) -> Option<&'static str> {
        let trimmed = code.trim_start();
        if trimmed.is_empty() {
            return None;
        }

        let lower = trimmed.to_ascii_lowercase();
        let first_line = trimmed.lines().next().unwrap_or_default().trim();
        if first_line.starts_with("#!") {
            if first_line.contains("python") {
                return Some("python");
            }
            if first_line.contains("node") {
                return Some("javascript");
            }
            if first_line.contains("bash")
                || first_line.contains("/sh")
                || first_line.ends_with(" sh")
            {
                return Some("bash");
            }
        }

        if trimmed.starts_with('{') && lower.contains("\"nbformat\"") && lower.contains("\"cells\"")
        {
            return Some("jupyter");
        }

        let python_score = trimmed
            .lines()
            .map(str::trim_start)
            .map(|line| {
                usize::from(line.starts_with("import "))
                    + usize::from(line.starts_with("from ") && line.contains(" import "))
                    + usize::from(line.starts_with("def "))
                    + usize::from(line.starts_with("async def "))
                    + usize::from(line.starts_with("class "))
                    + usize::from(line.starts_with("if __name__"))
            })
            .sum::<usize>()
            + usize::from(lower.contains("print("))
            + usize::from(lower.contains("subprocess."))
            + usize::from(lower.contains("requests."))
            + usize::from(lower.contains("httpx."))
            + usize::from(lower.contains("feedparser."))
            + usize::from(lower.contains("cv2."))
            + usize::from(lower.contains("pandas."))
            + usize::from(lower.contains("numpy."));

        let node_score = trimmed
            .lines()
            .map(str::trim_start)
            .map(|line| {
                usize::from(line.starts_with("const "))
                    + usize::from(line.starts_with("let "))
                    + usize::from(line.starts_with("var "))
                    + usize::from(line.starts_with("function "))
                    + usize::from(line.starts_with("import ") && line.contains(" from "))
                    + usize::from(line.starts_with("export "))
            })
            .sum::<usize>()
            + usize::from(lower.contains("console.log("))
            + usize::from(lower.contains("require("))
            + usize::from(lower.contains("process."))
            + usize::from(lower.contains("json.stringify("))
            + usize::from(lower.contains("=>"));

        let bash_score = trimmed
            .lines()
            .map(str::trim_start)
            .map(|line| {
                usize::from(line.starts_with("set -e"))
                    + usize::from(line.starts_with("cd "))
                    + usize::from(line.starts_with("curl "))
                    + usize::from(line.starts_with("wget "))
                    + usize::from(line.starts_with("printf "))
                    + usize::from(line.starts_with("echo "))
            })
            .sum::<usize>()
            + usize::from(lower.contains(" fi\n"))
            + usize::from(lower.contains(" done\n"));

        let best = [
            ("python", python_score),
            ("javascript", node_score),
            ("bash", bash_score),
        ]
        .into_iter()
        .max_by_key(|(_, score)| *score)?;

        if best.1 == 0 { None } else { Some(best.0) }
    }

    pub(super) async fn normalize_action_arguments(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        fallback_text: &str,
    ) -> std::result::Result<serde_json::Value, String> {
        let Some(action) = self
            .runtime
            .list_enabled_actions()
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|candidate| candidate.name == action_name)
        else {
            return Err(format!("Action `{}` is not available.", action_name));
        };

        let mut payload = arguments.as_object().cloned().unwrap_or_default();

        // Code execution can derive language from structured code content when
        // the schema needs it and the caller omitted the field.
        if action_name == "code_execute"
            && !Self::required_action_argument_present(payload.get("language"))
        {
            if let Some(language) = payload
                .get("code")
                .and_then(|value| value.as_str())
                .and_then(Self::infer_code_execute_language_from_code)
            {
                payload.insert(
                    "language".to_string(),
                    serde_json::Value::String(language.to_string()),
                );
            }
        }

        // Action-agnostic structural fallback: any required field literally
        // named `query` may be filled from the original fallback_text. This is
        // a schema-shape rule, not phrasing logic.
        let post_inference_missing = missing_required_fields(&action, &payload);
        if post_inference_missing.iter().any(|field| field == "query") {
            let fallback_query = fallback_text.trim();
            if !fallback_query.is_empty() {
                payload.insert(
                    "query".to_string(),
                    serde_json::Value::String(fallback_query.to_string()),
                );
            }
        }

        let final_missing = missing_required_fields(&action, &payload);
        if !final_missing.is_empty() {
            return Err(format!(
                "Action `{}` is missing required field(s): {}. Retry the tool call with those fields.",
                action_name,
                final_missing.join(", ")
            ));
        }

        Ok(serde_json::Value::Object(payload))
    }

    pub(super) fn extract_structured_watcher_condition_payload(
        result: &str,
    ) -> Option<serde_json::Value> {
        let trimmed = result.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(payload) = trimmed
            .trim_start()
            .strip_prefix(crate::runtime::TOOL_COMPLETION_MARKER)
        {
            let payload = payload.lines().next().unwrap_or(payload).trim();
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
                if let Some(data) = value.get("data") {
                    return Some(data.clone());
                }
                if value.is_object() {
                    return Some(value);
                }
            }
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(output) = value.get("output").and_then(|value| value.as_str()) {
                if let Some(inner) = extract_json_object_from_text(output) {
                    if let Some(data) = inner.get("data") {
                        return Some(data.clone());
                    }
                    return Some(inner);
                }
            }
            if let Some(data) = value.get("data") {
                return Some(data.clone());
            }
            if value.is_object() {
                return Some(value);
            }
        }
        extract_json_object_from_text(trimmed)
            .map(|value| value.get("data").cloned().unwrap_or(value))
    }

    pub(super) fn compare_structured_condition_number(
        actual: f64,
        op: &str,
        expected: f64,
    ) -> bool {
        match op {
            ">" => actual > expected,
            ">=" => actual >= expected,
            "<" => actual < expected,
            "<=" => actual <= expected,
            "=" | "==" => (actual - expected).abs() <= f64::EPSILON,
            _ => false,
        }
    }

    pub(super) fn json_value_at_watch_condition_path<'a>(
        root: &'a serde_json::Value,
        path: &str,
    ) -> Option<&'a serde_json::Value> {
        let trimmed = path.trim();
        if trimmed.is_empty() || trimmed == "$" || trimmed == "." {
            return Some(root);
        }
        let normalized = trimmed.trim_start_matches('$').trim_start_matches('.');
        if normalized.is_empty() {
            return Some(root);
        }
        let mut current = root;
        for segment in normalized
            .split('.')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
        {
            if let Ok(index) = segment.parse::<usize>() {
                current = current.as_array()?.get(index)?;
            } else {
                current = current.get(segment)?;
            }
        }
        Some(current)
    }

    pub(super) fn json_value_is_non_empty(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Null => false,
            serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
            serde_json::Value::String(text) => !text.trim().is_empty(),
            serde_json::Value::Array(items) => !items.is_empty(),
            serde_json::Value::Object(map) => !map.is_empty(),
        }
    }

    pub(super) fn json_value_as_f64(value: &serde_json::Value) -> Option<f64> {
        value.as_f64().or_else(|| {
            value
                .as_str()
                .and_then(|text| text.trim().parse::<f64>().ok())
        })
    }

    pub(super) fn json_value_as_bool(value: &serde_json::Value) -> Option<bool> {
        value.as_bool().or_else(|| {
            value.as_str().and_then(|text| {
                let lower = text.trim().to_ascii_lowercase();
                match lower.as_str() {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                }
            })
        })
    }

    pub(super) fn json_values_equal_relaxed(
        actual: &serde_json::Value,
        expected: &serde_json::Value,
    ) -> bool {
        match (actual, expected) {
            (serde_json::Value::String(left), serde_json::Value::String(right)) => {
                left.eq_ignore_ascii_case(right)
            }
            (serde_json::Value::Number(_), serde_json::Value::Number(_)) => {
                Self::json_value_as_f64(actual)
                    .zip(Self::json_value_as_f64(expected))
                    .map(|(left, right)| (left - right).abs() <= f64::EPSILON)
                    .unwrap_or(false)
            }
            (serde_json::Value::Bool(left), serde_json::Value::Bool(right)) => left == right,
            _ => actual == expected,
        }
    }

    pub(super) fn json_value_contains_expected(
        actual: &serde_json::Value,
        expected: &serde_json::Value,
    ) -> bool {
        match (actual, expected) {
            (serde_json::Value::String(left), serde_json::Value::String(right)) => left
                .to_ascii_lowercase()
                .contains(&right.to_ascii_lowercase()),
            (serde_json::Value::Array(items), serde_json::Value::Array(expected_items)) => {
                expected_items.iter().all(|expected_item| {
                    items.iter().any(|item| {
                        Self::json_values_equal_relaxed(item, expected_item)
                            || Self::json_value_contains_expected(item, expected_item)
                    })
                })
            }
            (serde_json::Value::Array(items), _) => items.iter().any(|item| {
                Self::json_values_equal_relaxed(item, expected)
                    || Self::json_value_contains_expected(item, expected)
            }),
            (serde_json::Value::Object(fields), serde_json::Value::String(_)) => fields
                .values()
                .any(|value| Self::json_value_contains_expected(value, expected)),
            (serde_json::Value::Object(fields), serde_json::Value::Array(expected_items)) => {
                expected_items.iter().all(|expected_item| {
                    fields
                        .values()
                        .any(|value| Self::json_value_contains_expected(value, expected_item))
                })
            }
            (serde_json::Value::Object(fields), _) => fields.values().any(|value| {
                Self::json_values_equal_relaxed(value, expected)
                    || Self::json_value_contains_expected(value, expected)
            }),
            (
                serde_json::Value::Number(_) | serde_json::Value::Bool(_),
                serde_json::Value::String(right),
            ) => actual
                .to_string()
                .to_ascii_lowercase()
                .contains(&right.to_ascii_lowercase()),
            _ => false,
        }
    }

    pub(super) fn evaluate_watch_json_predicate(
        payload: &serde_json::Value,
        predicate: &crate::core::watcher::WatchJsonPredicate,
    ) -> Result<bool, String> {
        let path = if predicate.path.trim().is_empty() {
            "$"
        } else {
            predicate.path.trim()
        };
        let target = Self::json_value_at_watch_condition_path(payload, &predicate.path);
        match predicate.operator {
            crate::core::watcher::WatchConditionOperator::Exists => return Ok(target.is_some()),
            crate::core::watcher::WatchConditionOperator::NotExists => return Ok(target.is_none()),
            _ => {}
        }

        let Some(target) = target else {
            return Err(format!(
                "condition path `{}` was not present in the poll output",
                path
            ));
        };

        match predicate.operator {
            crate::core::watcher::WatchConditionOperator::Eq => {
                Ok(Self::json_values_equal_relaxed(
                    target,
                    predicate.value.as_ref().ok_or_else(|| {
                        format!("condition path `{}` is missing comparison value", path)
                    })?,
                ))
            }
            crate::core::watcher::WatchConditionOperator::Ne => {
                Ok(!Self::json_values_equal_relaxed(
                    target,
                    predicate.value.as_ref().ok_or_else(|| {
                        format!("condition path `{}` is missing comparison value", path)
                    })?,
                ))
            }
            crate::core::watcher::WatchConditionOperator::Gt
            | crate::core::watcher::WatchConditionOperator::Gte
            | crate::core::watcher::WatchConditionOperator::Lt
            | crate::core::watcher::WatchConditionOperator::Lte => {
                let actual = Self::json_value_as_f64(target).ok_or_else(|| {
                    format!(
                        "condition path `{}` is not numeric in the poll output",
                        path
                    )
                })?;
                let expected = predicate
                    .value
                    .as_ref()
                    .and_then(Self::json_value_as_f64)
                    .ok_or_else(|| {
                        format!(
                            "condition path `{}` is missing numeric comparison value",
                            path
                        )
                    })?;
                let operator = match predicate.operator {
                    crate::core::watcher::WatchConditionOperator::Gt => ">",
                    crate::core::watcher::WatchConditionOperator::Gte => ">=",
                    crate::core::watcher::WatchConditionOperator::Lt => "<",
                    crate::core::watcher::WatchConditionOperator::Lte => "<=",
                    _ => unreachable!(),
                };
                Ok(Self::compare_structured_condition_number(
                    actual, operator, expected,
                ))
            }
            crate::core::watcher::WatchConditionOperator::Contains => {
                Ok(Self::json_value_contains_expected(
                    target,
                    predicate.value.as_ref().ok_or_else(|| {
                        format!("condition path `{}` is missing contains value", path)
                    })?,
                ))
            }
            crate::core::watcher::WatchConditionOperator::NotContains => {
                Ok(!Self::json_value_contains_expected(
                    target,
                    predicate.value.as_ref().ok_or_else(|| {
                        format!("condition path `{}` is missing contains value", path)
                    })?,
                ))
            }
            crate::core::watcher::WatchConditionOperator::NonEmpty => {
                Ok(Self::json_value_is_non_empty(target))
            }
            crate::core::watcher::WatchConditionOperator::Empty => {
                Ok(!Self::json_value_is_non_empty(target))
            }
            crate::core::watcher::WatchConditionOperator::True => {
                Ok(Self::json_value_as_bool(target).ok_or_else(|| {
                    format!(
                        "condition path `{}` is not boolean-like in the poll output",
                        path
                    )
                })?)
            }
            crate::core::watcher::WatchConditionOperator::False => {
                Ok(!Self::json_value_as_bool(target).ok_or_else(|| {
                    format!(
                        "condition path `{}` is not boolean-like in the poll output",
                        path
                    )
                })?)
            }
            crate::core::watcher::WatchConditionOperator::Regex => {
                let pattern = predicate
                    .value
                    .as_ref()
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| format!("condition path `{}` is missing regex pattern", path))?;
                let target_text = target.as_str().ok_or_else(|| {
                    format!(
                        "condition path `{}` is not a string in the poll output",
                        path
                    )
                })?;
                let regex = Regex::new(pattern)
                    .map_err(|error| format!("condition regex is invalid: {}", error))?;
                Ok(regex.is_match(target_text))
            }
            crate::core::watcher::WatchConditionOperator::Exists
            | crate::core::watcher::WatchConditionOperator::NotExists => unreachable!(),
        }
    }

    fn normalized_watch_result_for_change_detection(result: &str) -> String {
        if let Some(payload) = Self::extract_structured_watcher_condition_payload(result) {
            return serde_json::to_string(&payload).unwrap_or_else(|_| payload.to_string());
        }
        automation_primary_result_text(result)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub(crate) fn evaluate_watch_condition_without_llm(
        condition: &crate::core::watcher::WatchCondition,
        result: &str,
        previous_result: Option<&str>,
    ) -> Option<Result<bool, String>> {
        let primary = automation_primary_result_text(result);
        let trimmed = primary.trim();
        let current_state_match = match &condition.matcher {
            crate::core::watcher::WatchConditionMatcher::NotEmpty => Some(Ok(!trimmed.is_empty()
                && !trimmed.eq_ignore_ascii_case("no messages found.")
                && !trimmed.eq_ignore_ascii_case("no results")
                && !trimmed.eq_ignore_ascii_case("no results found")
                && !trimmed.starts_with("Error"))),
            crate::core::watcher::WatchConditionMatcher::TextContains {
                text,
                case_sensitive,
            } => Some(Ok(if *case_sensitive {
                trimmed.contains(text)
            } else {
                trimmed
                    .to_ascii_lowercase()
                    .contains(&text.to_ascii_lowercase())
            })),
            crate::core::watcher::WatchConditionMatcher::Regex { pattern } => Some(
                Regex::new(pattern)
                    .map(|regex| regex.is_match(trimmed))
                    .map_err(|error| format!("condition regex is invalid: {}", error)),
            ),
            crate::core::watcher::WatchConditionMatcher::JsonPredicate {
                path,
                operator,
                value,
            } => {
                let payload = Self::extract_structured_watcher_condition_payload(result)
                    .ok_or_else(|| {
                        "poll result did not contain structured JSON output".to_string()
                    });
                Some(payload.and_then(|payload| {
                    Self::evaluate_watch_json_predicate(
                        &payload,
                        &crate::core::watcher::WatchJsonPredicate {
                            path: path.clone(),
                            operator: operator.clone(),
                            value: value.clone(),
                        },
                    )
                }))
            }
            crate::core::watcher::WatchConditionMatcher::JsonLogic { logic, rules } => {
                let payload = Self::extract_structured_watcher_condition_payload(result)
                    .ok_or_else(|| {
                        "poll result did not contain structured JSON output".to_string()
                    });
                Some(payload.and_then(|payload| match logic {
                    crate::core::watcher::WatchConditionLogic::All => {
                        for rule in rules {
                            if !Self::evaluate_watch_json_predicate(&payload, rule)? {
                                return Ok(false);
                            }
                        }
                        Ok(true)
                    }
                    crate::core::watcher::WatchConditionLogic::Any => {
                        let mut last_error: Option<String> = None;
                        let mut had_evaluable_rule = false;
                        for rule in rules {
                            match Self::evaluate_watch_json_predicate(&payload, rule) {
                                Ok(true) => return Ok(true),
                                Ok(false) => had_evaluable_rule = true,
                                Err(error) => last_error = Some(error),
                            }
                        }
                        if had_evaluable_rule {
                            Ok(false)
                        } else {
                            Err(last_error.unwrap_or_else(|| {
                                "watcher json_logic condition did not have any evaluable rules"
                                    .to_string()
                            }))
                        }
                    }
                }))
            }
            crate::core::watcher::WatchConditionMatcher::Llm => None,
        }?;

        Some(current_state_match.map(|matched| {
            if !matched {
                return false;
            }
            if condition.evaluation_mode
                != crate::core::watcher::WatchConditionEvaluationMode::Change
            {
                return true;
            }
            let Some(previous_result) = previous_result else {
                return false;
            };
            Self::normalized_watch_result_for_change_detection(result)
                != Self::normalized_watch_result_for_change_detection(previous_result)
        }))
    }

    fn watch_condition_requires_previous_result(
        condition: &crate::core::watcher::WatchCondition,
    ) -> bool {
        condition.evaluation_mode == crate::core::watcher::WatchConditionEvaluationMode::Change
    }

    pub async fn evaluate_watcher_condition(
        &self,
        watcher_description: &str,
        condition: &crate::core::watcher::WatchCondition,
        result: &str,
        previous_result: Option<&str>,
    ) -> Result<bool, String> {
        if Self::watch_condition_requires_previous_result(condition) && previous_result.is_none() {
            return Ok(false);
        }

        if let Some(outcome) =
            Self::evaluate_watch_condition_without_llm(condition, result, previous_result)
        {
            return outcome;
        }

        let prompt = serde_json::json!({
            "watcher_description": safe_truncate(watcher_description.trim(), 280),
            "condition": condition,
            "poll_result": safe_truncate(automation_primary_result_text(result).trim(), 4000),
            "previous_poll_result": previous_result
                .map(automation_primary_result_text)
                .map(|value| safe_truncate(value.trim(), 4000)),
        });

        let response = match self
            .supervised_internal_chat(
                "watcher",
                "custom_condition",
                "custom_watcher_condition",
                &ModelRole::Fast,
                vec![],
                "You decide whether a watcher poll result satisfies a watcher condition contract. Return strict JSON only in the shape {\"matched\": true|false, \"reason\": \"short string\"}. If the condition is about change, compare poll_result with previous_poll_result and return matched=false when no previous result exists or the difference is not material. Be conservative: if the result is ambiguous, stale, routine, or does not clearly satisfy the condition, return matched=false.",
                &prompt.to_string(),
                &[],
                &[],
                internal_llm_timeout_ms("AGENTARK_WATCHER_CONDITION_TIMEOUT_MS", 20_000),
                2,
            )
            .await
        {
            Some(response) => response,
            None => {
                return Err(format!(
                    "watcher condition evaluation failed for '{}': exhausted eligible model attempts",
                    watcher_description
                ));
            }
        };

        let Some(parsed) = extract_json_object_from_text(&response.content) else {
            return Err("watcher condition evaluator returned malformed JSON".to_string());
        };
        Ok(parsed
            .get("matched")
            .and_then(|value| value.as_bool())
            .unwrap_or(false))
    }

    pub(super) fn watch_condition_requires_structured_payload(
        condition: &crate::core::watcher::WatchCondition,
    ) -> bool {
        matches!(
            condition.matcher,
            crate::core::watcher::WatchConditionMatcher::JsonPredicate { .. }
                | crate::core::watcher::WatchConditionMatcher::JsonLogic { .. }
        )
    }

    pub async fn add_or_update_similar_task(
        &self,
        mut task: super::task::Task,
        allow_duplicate: bool,
        target_task_id: Option<uuid::Uuid>,
    ) -> Result<(uuid::Uuid, bool, usize)> {
        task.approval = super::task::normalized_task_approval(&task.approval);
        if super::task::task_requires_explicit_approval(&task.approval)
            && matches!(
                task.status,
                super::task::TaskStatus::Pending | super::task::TaskStatus::AwaitingApproval
            )
        {
            task.status = super::task::TaskStatus::AwaitingApproval;
        }
        if let Some(target_id) = target_task_id {
            let kept_task = {
                let mut queue = self.tasks.write().await;
                let existing = queue.get_mut(target_id).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Task `{}` was not found. Use `list_tasks` to choose an existing task.",
                        target_id
                    )
                })?;
                if matches!(existing.status, super::task::TaskStatus::Completed) {
                    return Err(anyhow::anyhow!(
                        "Task `{}` is already completed. Create a new task or choose an active routine.",
                        target_id
                    ));
                }
                let preserved_status = existing.status.clone();
                existing.description = task.description.clone();
                existing.action = task.action.clone();
                existing.arguments = task.arguments.clone();
                existing.approval = task.approval.clone();
                existing.capabilities = task.capabilities.clone();
                existing.status = if matches!(preserved_status, super::task::TaskStatus::InProgress)
                {
                    super::task::TaskStatus::InProgress
                } else {
                    task.status.clone()
                };
                existing.created_at = chrono::Utc::now();
                existing.scheduled_for = task.scheduled_for;
                existing.cron = task.cron.clone();
                existing.result = None;
                existing.proof_id = None;
                existing.priority = task.priority;
                existing.urgency = task.urgency;
                existing.importance = task.importance;
                existing.eisenhower_quadrant = task.eisenhower_quadrant;
                existing.clone()
            };
            let scheduled_for = kept_task.scheduled_for.as_ref().map(|dt| dt.to_rfc3339());
            let status_json = serde_json::to_string(&kept_task.status)
                .unwrap_or_else(|_| "\"Pending\"".to_string());
            self.storage
                .retry_task(
                    &kept_task.id.to_string(),
                    &status_json,
                    scheduled_for.clone(),
                )
                .await?;
            let args_json = serde_json::to_string(&kept_task.arguments).ok();
            self.storage
                .update_task(
                    &kept_task.id.to_string(),
                    Some(kept_task.description.clone()),
                    args_json,
                    None,
                    None,
                )
                .await?;
            self.storage
                .replace_task_schedule(
                    &kept_task.id.to_string(),
                    kept_task.cron.clone(),
                    scheduled_for,
                )
                .await?;
            if matches!(kept_task.status, super::task::TaskStatus::AwaitingApproval) {
                self.register_task_approval_request(&kept_task).await?;
            }
            return Ok((kept_task.id, true, 0));
        }
        if allow_duplicate {
            self.storage.insert_task(&task).await?;
            self.tasks.write().await.add(task.clone());
            if matches!(task.status, super::task::TaskStatus::AwaitingApproval) {
                self.register_task_approval_request(&task).await?;
            }
            return Ok((task.id, false, 0));
        }

        let mut kept_task: Option<super::task::Task> = None;
        let mut removed_duplicate_ids: Vec<uuid::Uuid> = Vec::new();
        let mut reused_existing = false;

        {
            let mut queue = self.tasks.write().await;
            let matching_ids = queue
                .all()
                .iter()
                .filter(|existing| {
                    !matches!(existing.status, super::task::TaskStatus::Completed)
                        && crate::core::task::tasks_are_semantically_similar(existing, &task)
                })
                .map(|existing| existing.id)
                .collect::<Vec<_>>();

            if let Some(keeper_id) = matching_ids.first().copied() {
                reused_existing = true;
                let preserved_status = queue
                    .all()
                    .iter()
                    .find(|existing| existing.id == keeper_id)
                    .map(|existing| existing.status.clone())
                    .unwrap_or(super::task::TaskStatus::Pending);
                if let Some(existing) = queue.get_mut(keeper_id) {
                    existing.description = task.description.clone();
                    existing.action = task.action.clone();
                    existing.arguments = task.arguments.clone();
                    existing.approval = task.approval.clone();
                    existing.capabilities = task.capabilities.clone();
                    existing.status =
                        if matches!(preserved_status, super::task::TaskStatus::InProgress) {
                            super::task::TaskStatus::InProgress
                        } else {
                            task.status.clone()
                        };
                    existing.created_at = chrono::Utc::now();
                    existing.scheduled_for = task.scheduled_for;
                    existing.cron = task.cron.clone();
                    existing.result = None;
                    existing.proof_id = None;
                    existing.priority = task.priority;
                    existing.urgency = task.urgency;
                    existing.importance = task.importance;
                    existing.eisenhower_quadrant = task.eisenhower_quadrant;
                    kept_task = Some(existing.clone());
                }
                for duplicate_id in matching_ids.iter().skip(1) {
                    if queue.remove(*duplicate_id) {
                        removed_duplicate_ids.push(*duplicate_id);
                    }
                }
            } else {
                queue.add(task.clone());
                kept_task = Some(task);
            }
        }

        let kept_task = kept_task.ok_or_else(|| anyhow::anyhow!("Failed to upsert task state"))?;
        let scheduled_for = kept_task.scheduled_for.as_ref().map(|dt| dt.to_rfc3339());
        if reused_existing {
            let status_json = serde_json::to_string(&kept_task.status)
                .unwrap_or_else(|_| "\"Pending\"".to_string());
            self.storage
                .retry_task(
                    &kept_task.id.to_string(),
                    &status_json,
                    scheduled_for.clone(),
                )
                .await?;
            let args_json = serde_json::to_string(&kept_task.arguments).ok();
            self.storage
                .update_task(
                    &kept_task.id.to_string(),
                    Some(kept_task.description.clone()),
                    args_json,
                    None,
                    None,
                )
                .await?;
            self.storage
                .replace_task_schedule(
                    &kept_task.id.to_string(),
                    kept_task.cron.clone(),
                    scheduled_for,
                )
                .await?;
        } else {
            self.storage.insert_task(&kept_task).await?;
        }
        for duplicate_id in &removed_duplicate_ids {
            let _ = self.storage.delete_task(&duplicate_id.to_string()).await;
        }
        if matches!(kept_task.status, super::task::TaskStatus::AwaitingApproval) {
            self.register_task_approval_request(&kept_task).await?;
        }

        Ok((kept_task.id, reused_existing, removed_duplicate_ids.len()))
    }

    pub(super) async fn authorize_automation_tool_call(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        authorization: Option<&crate::actions::ActionAuthorizationContext>,
    ) -> std::result::Result<(), String> {
        let default_authorization = crate::actions::ActionAuthorizationContext::default();
        let authorization = authorization.unwrap_or(&default_authorization);
        let action_def = self.runtime.action_definition(action_name).await;
        match self
            .runtime
            .authorize_action_invocation(action_name, action_def.as_ref(), arguments, authorization)
            .await
        {
            Ok(decision) if decision.allowed => Ok(()),
            Ok(decision) => Err(decision.reason),
            Err(error) => Err(format!(
                "Failed to authorize '{}' execution: {}",
                action_name, error
            )),
        }
    }

    /// Handle schedule_task tool call - actually create the scheduled task
    pub(super) async fn handle_schedule_task(
        &self,
        arguments: &serde_json::Value,
        request_channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        authorization: Option<&crate::actions::ActionAuthorizationContext>,
    ) -> Option<String> {
        if let Some(batch_items) = schedule_task_batch_item_arguments(arguments) {
            let batch_items = match batch_items {
                Ok(items) => items,
                Err(error) => return Some(error.to_string()),
            };
            for item_args in &batch_items {
                if let Err(message) = self
                    .authorize_automation_tool_call("schedule_task", item_args, authorization)
                    .await
                {
                    return Some(message);
                }
            }

            let mut task_records = Vec::new();
            let mut summary_lines = Vec::new();
            for (index, item_args) in batch_items.iter().enumerate() {
                let Some(result) = Box::pin(self.handle_schedule_task(
                    item_args,
                    request_channel,
                    conversation_id,
                    project_id,
                    authorization,
                ))
                .await
                else {
                    return Some(format!(
                        "Failed to schedule item {} of {}.",
                        index + 1,
                        batch_items.len()
                    ));
                };
                let Some(data) = schedule_task_completion_data(&result) else {
                    let readable = strip_tool_completion_marker_line(&result);
                    if task_records.is_empty() {
                        return Some(readable);
                    }
                    return Some(format!(
                        "Saved {} of {} scheduled item(s), then item {} needed attention:\n{}",
                        task_records.len(),
                        batch_items.len(),
                        index + 1,
                        readable
                    ));
                };
                let task_title = data
                    .get("task")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Scheduled task");
                let schedule = data
                    .get("schedule")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("scheduled time");
                summary_lines.push(format!("- {} ({})", task_title, schedule));
                task_records.push(data);
            }

            let object_refs = task_records
                .iter()
                .filter_map(|record| {
                    record
                        .get("task_id")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|id| serde_json::json!({ "kind": "task", "id": id }))
                })
                .collect::<Vec<_>>();
            let count = task_records.len();
            let completion_detail = format!("Saved {} scheduled task(s).", count);
            return Some(format!(
                "{}\n{}\n{}",
                render_tool_completion_marker_with_data(
                    "schedule_task",
                    "completed",
                    &completion_detail,
                    serde_json::json!({
                        "task_count": count,
                        "tasks": task_records,
                        "object_refs": object_refs,
                    }),
                ),
                completion_detail,
                summary_lines.join("\n")
            ));
        }

        if let Err(message) = self
            .authorize_automation_tool_call("schedule_task", arguments, authorization)
            .await
        {
            return Some(message);
        }
        let allow_duplicate = arguments
            .get("allow_duplicate")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let explicit_task_id = match arguments
            .get("task_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(value) => match uuid::Uuid::parse_str(value) {
                Ok(id) => Some(id),
                Err(error) => {
                    return Some(format!(
                        "Invalid task_id `{}`: {}. Use `list_tasks` and retry with the task ID.",
                        value, error
                    ));
                }
            },
            None => None,
        };
        let existing_task_target = match explicit_task_id {
            Some(id) => {
                let task = {
                    let tasks = self.tasks.read().await;
                    tasks.all().iter().find(|task| task.id == id).cloned()
                };
                match task {
                    Some(task) => Some(task),
                    None => {
                        return Some(format!(
                            "Task `{}` was not found. Use `list_tasks` and retry with an existing task ID.",
                            id
                        ));
                    }
                }
            }
            None => None,
        };
        let task_desc =
            schedule_task_description_from_arguments(arguments, existing_task_target.as_ref());
        let Some(task_desc) = task_desc else {
            return Some(schedule_task_validation_failure_result(
                "Task scheduling requires `task` unless `task_id` points at an existing task.",
                "missing_task",
            ));
        };

        // Parse cron or at time.
        let default_timezone = {
            let profile = self.user_profile.read().await;
            profile
                .timezone
                .as_deref()
                .and_then(|value| value.parse::<chrono_tz::Tz>().ok())
        };
        let schedule_context = ScheduleTaskScheduleContext {
            now_utc: chrono::Utc::now(),
            existing_task_target: existing_task_target.as_ref(),
            default_timezone,
        };
        let (cron_expr, scheduled_for) =
            match schedule_task_schedule_from_arguments(arguments, schedule_context) {
                Ok(schedule) => schedule,
                Err(error) => {
                    return Some(schedule_task_validation_failure_result(
                        &error,
                        "invalid_schedule",
                    ));
                }
            };
        if let Some(cron) = cron_expr.as_deref() {
            let fields = cron.split_whitespace().collect::<Vec<_>>();
            let requests_subminute = fields.len() == 6 && fields.first().copied() != Some("0");
            if requests_subminute {
                return Some(
                    "Sub-minute recurring schedules are not supported by schedule_task. Use a managed service or background command loop when the workflow genuinely needs sub-minute polling."
                        .to_string(),
                );
            }
        }

        let report_to = normalize_automation_notification_channel(
            arguments
                .get("report_to")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    existing_task_target.as_ref().and_then(|task| {
                        task.arguments
                            .get("report_to")
                            .and_then(|value| value.as_str())
                    })
                }),
        );

        let script_action_arguments = arguments
            .get("script")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|script| {
                let language = arguments
                    .get("script_language")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("python");
                serde_json::json!({
                    "language": language,
                    "code": script,
                    "network_access": arguments.get("network_access").and_then(|value| value.as_bool()).unwrap_or(false),
                    "execution_contract": {
                        "phase": "poll",
                        "target_validated_when_successful": true
                    },
                    "context_from": arguments.get("context_from").cloned().unwrap_or_else(|| serde_json::json!([])),
                    "workdir": arguments.get("workdir").cloned().unwrap_or(serde_json::Value::Null),
                })
            });

        let explicit_action = arguments
            .get("action")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                existing_task_target
                    .as_ref()
                    .map(|task| task.action.clone())
                    .or_else(|| {
                        script_action_arguments
                            .as_ref()
                            .map(|_| "code_execute".to_string())
                    })
            });

        // Build task arguments: start with explicit action_arguments if provided.
        let mut task_args = arguments
            .get("action_arguments")
            .cloned()
            .or_else(|| script_action_arguments.clone())
            .or_else(|| {
                existing_task_target
                    .as_ref()
                    .map(|task| task.arguments.clone())
            })
            .unwrap_or_else(|| serde_json::json!({}));
        if !task_args.is_object() {
            task_args = serde_json::json!({});
        }
        if let Some(task_args_obj) = task_args.as_object_mut() {
            if !task_args_obj.contains_key("query") {
                task_args_obj.insert(
                    "query".to_string(),
                    serde_json::Value::String(task_desc.to_string()),
                );
            }
            if !task_args_obj.contains_key("report_to") {
                task_args_obj.insert(
                    "report_to".to_string(),
                    serde_json::Value::String(report_to.clone()),
                );
            }
        }
        let all_actions = self
            .runtime
            .list_enabled_actions()
            .await
            .unwrap_or_default();

        let explicit_valid = explicit_action
            .as_ref()
            .map(|name| all_actions.iter().any(|a| a.name == *name))
            .unwrap_or(false);

        if explicit_action.is_some() && !explicit_valid {
            return Some(format!(
                "Scheduled task action `{}` is not available in the enabled action catalog. Use an enabled action or omit `action` for an in-app notification reminder.",
                explicit_action.as_deref().unwrap_or_default()
            ));
        }

        let mut action_name = if explicit_valid {
            explicit_action.unwrap_or_default()
        } else if all_actions
            .iter()
            .any(|action| action.name == "notify_user")
        {
            "notify_user".to_string()
        } else {
            return Some(
                "Scheduled task is missing an enabled action and in-app notification delivery is unavailable."
                    .to_string(),
            );
        };
        let trigger_kind_hint = if cron_expr.is_some() {
            Some("recurring_schedule")
        } else if scheduled_for.is_some() {
            Some("absolute_date")
        } else {
            None
        };
        let validated_plan = self
            .validate_automation_plan(
                request_channel,
                AutomationSurface::Schedule,
                &task_desc,
                trigger_kind_hint,
                action_name.clone(),
                task_args.clone(),
                report_to.clone(),
                &all_actions,
            )
            .await;
        if let Some(reason) = validated_plan.blocked_reason {
            return Some(reason);
        }
        action_name = validated_plan.action_name;
        task_args = validated_plan.action_arguments;
        let report_to = validated_plan.delivery_channel;
        let planner_note = if validated_plan.notes.is_empty() {
            String::new()
        } else {
            format!(
                "\n\nPlanner note: {}",
                safe_truncate(&validated_plan.notes.join(" "), 220)
            )
        };
        task_args = match self
            .normalize_action_arguments(&action_name, &task_args, &task_desc)
            .await
        {
            Ok(normalized) => normalized,
            Err(error) => return Some(error),
        };
        let explicit_background_session_id =
            super::background_session::background_session_id_from_automation(&task_args);
        let background_session = self
            .ensure_background_session_for_automation(
                request_channel,
                conversation_id,
                project_id,
                explicit_background_session_id.as_deref(),
                &task_desc,
                "Wait for the scheduled task to execute and record the outcome.",
                background_session_policy_for_action(&all_actions, &action_name),
            )
            .await;
        let origin = AutomationOriginContext {
            channel: Some(request_channel.to_string()),
            conversation_id: conversation_id.map(|value| value.to_string()),
            project_id: project_id.map(|value| value.to_string()),
            source: Some("scheduled_task".to_string()),
        };
        let policy = automation_policy_from_request_argument(
            arguments,
            AutomationExecutionPolicy {
                validation: automation_validation_from_request_argument(
                    arguments,
                    default_automation_validation_for_action(&action_name),
                ),
                ..AutomationExecutionPolicy::default()
            },
        );
        task_args = inject_automation_context(&task_args, origin, policy);
        task_args = inject_automation_authorization_context(&task_args, authorization);
        let background_session_id = background_session
            .as_ref()
            .map(|session| session.id.as_str())
            .or(explicit_background_session_id.as_deref());
        task_args = super::background_session::set_background_session_id_in_automation(
            &task_args,
            background_session_id,
        );
        if let Err(error) = self
            .enforce_background_session_policy_for_action(&action_name, &task_args)
            .await
        {
            return Some(error);
        }
        let scheduled_auth = automation_runtime_authorization_context(
            &task_args,
            ActionExecutionSurface::Automation,
        );
        if let Err(error) = self
            .runtime
            .validate_action_invocation_with_context(&action_name, &task_args, &scheduled_auth)
            .await
        {
            return Some(format!(
                "Scheduled task action `{}` is not runnable yet: {}. Pick an action that can execute in automation with these arguments before creating the task.",
                action_name, error
            ));
        }

        let task = super::task::Task {
            id: uuid::Uuid::new_v4(),
            description: task_desc.to_string(),
            action: action_name.clone(),
            arguments: task_args,
            approval: super::task::TaskApproval::Auto,
            capabilities: vec![action_name.clone()],
            status: super::task::TaskStatus::Pending,
            created_at: chrono::Utc::now(),
            scheduled_for,
            cron: cron_expr.clone(),
            result: None,
            proof_id: None,
            priority: None,
            urgency: None,
            importance: None,
            eisenhower_quadrant: None,
        };

        if explicit_task_id.is_none() && !allow_duplicate {
            let existing_tasks = { self.tasks.read().await.all().to_vec() };
            if let Some(prompt) = Self::task_update_confirmation_prompt(&task, &existing_tasks) {
                return Some(prompt);
            }
        }

        let (task_id, reused_existing, removed_duplicates) = match self
            .add_or_update_similar_task(task, allow_duplicate, explicit_task_id)
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::error!("Failed to save scheduled task: {}", error);
                return Some(format!("Failed to schedule task: {}", error));
            }
        };

        let scheduled_for_text = scheduled_for.map(|value| value.to_rfc3339());
        let schedule_desc = if let Some(cron) = cron_expr.as_deref() {
            format!("cron {}", cron)
        } else if let Some(at) = scheduled_for_text.as_deref() {
            format!("one-time at {}", at)
        } else {
            "unknown".to_string()
        };
        let action_sentence = if reused_existing {
            "Updated the existing scheduled task."
        } else {
            "Scheduled the task."
        };
        let duplicate_note = if removed_duplicates > 0 {
            format!(
                "\nRemoved {} duplicate task(s) with the same purpose.",
                removed_duplicates
            )
        } else {
            String::new()
        };
        if let Some(cid) = conversation_id.filter(|value| !value.trim().is_empty()) {
            let task_id_text = task_id.to_string();
            let summary = if cron_expr.is_some() {
                "Recently scheduled recurring task in this conversation".to_string()
            } else {
                "Recently scheduled one-time task in this conversation".to_string()
            };
            self.persist_conversation_artifact_context(
                cid,
                ConversationArtifactSpec {
                    artifact_type: "task",
                    artifact_id: &task_id_text,
                    title: &task_desc,
                    summary: &summary,
                    url: None,
                    related_actions: &["list_tasks"],
                },
            )
            .await;
        }
        if let Some(session_id) = background_session_id {
            self.attach_items_to_background_session(session_id, &[task_id.to_string()], &[])
                .await;
        }
        let display_report_to = if report_to == "preferred" {
            background_session
                .as_ref()
                .and_then(|session| session.preferred_delivery_channel.clone())
                .unwrap_or(report_to.clone())
        } else {
            report_to.clone()
        };
        let report_label = watcher_delivery_label(&display_report_to);
        let completion_detail = format!(
            "{} It will run on a {} and report via {}.",
            action_sentence, schedule_desc, report_label
        );

        Some(format!(
            "{}\n{}\n\nTask: {}\nSchedule: {}\nReport to: {}{}{}",
            render_tool_completion_marker_with_data(
                "schedule_task",
                "completed",
                &completion_detail,
                serde_json::json!({
                    "kind": "task",
                    "task_id": task_id.to_string(),
                    "background_session_id": background_session_id,
                    "action": action_name.clone(),
                    "task": task_desc.clone(),
                    "schedule": schedule_desc.clone(),
                    "notification": report_label.clone(),
                    "cron": cron_expr.clone(),
                    "scheduled_for": scheduled_for_text.clone(),
                }),
            ),
            action_sentence,
            task_desc,
            schedule_desc,
            report_label,
            duplicate_note,
            planner_note
        ))
    }

    pub(super) fn watcher_supervisor_status_label(
        status: &super::watcher::WatcherStatus,
    ) -> String {
        match status {
            super::watcher::WatcherStatus::Active => "active".to_string(),
            super::watcher::WatcherStatus::Paused => "paused".to_string(),
            super::watcher::WatcherStatus::Triggered => "triggered".to_string(),
            super::watcher::WatcherStatus::TimedOut => "timed_out".to_string(),
            super::watcher::WatcherStatus::Cancelled => "cancelled".to_string(),
            super::watcher::WatcherStatus::Failed { .. } => "failed".to_string(),
        }
    }

    pub(super) fn task_status_debug_label(status: &super::task::TaskStatus) -> String {
        match status {
            super::task::TaskStatus::Pending => "pending".to_string(),
            super::task::TaskStatus::AwaitingApproval => "awaiting_approval".to_string(),
            super::task::TaskStatus::ExpiredNeedsReapproval => {
                "expired_needs_reapproval".to_string()
            }
            super::task::TaskStatus::Paused => "paused".to_string(),
            super::task::TaskStatus::InProgress => "in_progress".to_string(),
            super::task::TaskStatus::Completed => "completed".to_string(),
            super::task::TaskStatus::Failed { .. } => "failed".to_string(),
            super::task::TaskStatus::Cancelled => "cancelled".to_string(),
        }
    }

    pub async fn sync_watcher_supervisor_state(
        &self,
        watcher: &super::watcher::Watcher,
        status_override: Option<&str>,
        last_error_override: Option<String>,
    ) {
        let existing = load_automation_supervisor_state(&self.storage, &watcher.id.to_string())
            .await
            .ok()
            .flatten();
        let status = status_override
            .map(|value| value.to_string())
            .unwrap_or_else(|| Self::watcher_supervisor_status_label(&watcher.status));
        let origin = automation_origin_from_arguments(&watcher.poll_arguments);
        let created_at = existing
            .as_ref()
            .and_then(|state| state.created_at.clone())
            .unwrap_or_else(|| watcher.created_at.to_rfc3339());
        let last_run_at = watcher.last_poll_at.map(|ts| ts.to_rfc3339()).or_else(|| {
            existing
                .as_ref()
                .and_then(|state| state.last_run_at.clone())
        });
        let last_success_at = if status == "triggered" {
            Some(chrono::Utc::now().to_rfc3339())
        } else {
            existing
                .as_ref()
                .and_then(|state| state.last_success_at.clone())
        };
        let last_error = last_error_override
            .or_else(|| watcher.last_error.clone())
            .or_else(|| match &watcher.status {
                super::watcher::WatcherStatus::Failed { error } => Some(error.clone()),
                _ => None,
            })
            .or_else(|| {
                if matches!(status.as_str(), "active" | "paused" | "triggered") {
                    None
                } else {
                    existing.as_ref().and_then(|state| state.last_error.clone())
                }
            });
        let consecutive_failures =
            if matches!(status.as_str(), "failed" | "timed_out" | "cancelled") {
                existing
                    .as_ref()
                    .map(|state| state.consecutive_failures)
                    .unwrap_or(0)
                    .max(1)
            } else {
                watcher.consecutive_failures
            };

        let state = AutomationSupervisorState {
            automation_id: watcher.id.to_string(),
            automation_kind: "watcher".to_string(),
            title: watcher.description.clone(),
            action: watcher.poll_action.clone(),
            status,
            attempt_count: watcher.poll_count,
            consecutive_failures,
            last_run_id: existing
                .as_ref()
                .and_then(|state| state.last_run_id.clone()),
            last_run_at,
            last_success_at,
            last_error,
            next_retry_at: watcher.next_poll_not_before.map(|ts| ts.to_rfc3339()),
            stalled_count: existing
                .as_ref()
                .map(|state| state.stalled_count)
                .unwrap_or(0),
            origin,
            created_at: Some(created_at),
        };
        if let Err(error) = upsert_automation_supervisor_state(&self.storage, state).await {
            tracing::warn!(
                "Failed to sync watcher supervisor state for '{}': {}",
                watcher.id,
                error
            );
        }
    }

    pub async fn clear_watcher_supervisor_state(&self, watcher_id: &str) -> bool {
        delete_automation_supervisor_state(&self.storage, watcher_id)
            .await
            .unwrap_or(false)
    }

    /// Handle watch tool call - create a background watcher
    pub async fn handle_watch(
        &self,
        arguments: &serde_json::Value,
        request_channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        authorization: Option<&crate::actions::ActionAuthorizationContext>,
    ) -> Option<String> {
        if let Some(batch_items) = watch_batch_item_arguments(arguments) {
            let batch_items = match batch_items {
                Ok(items) => items,
                Err(error) => return Some(error.to_string()),
            };
            for item_args in &batch_items {
                if let Err(message) = self
                    .authorize_automation_tool_call("watch", item_args, authorization)
                    .await
                {
                    return Some(message);
                }
            }

            let mut watcher_records = Vec::new();
            let mut summary_lines = Vec::new();
            for (index, item_args) in batch_items.iter().enumerate() {
                let Some(result) = Box::pin(self.handle_watch(
                    item_args,
                    request_channel,
                    conversation_id,
                    project_id,
                    authorization,
                ))
                .await
                else {
                    return Some(format!(
                        "Failed to create watcher item {} of {}.",
                        index + 1,
                        batch_items.len()
                    ));
                };
                let Some(data) = schedule_task_completion_data(&result) else {
                    let readable = strip_tool_completion_marker_line(&result);
                    if watcher_records.is_empty() {
                        return Some(readable);
                    }
                    return Some(format!(
                        "Saved {} of {} watcher item(s), then item {} needed attention:\n{}",
                        watcher_records.len(),
                        batch_items.len(),
                        index + 1,
                        readable
                    ));
                };
                let description = data
                    .get("description")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Watcher");
                let cadence = data
                    .get("cadence")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("scheduled cadence");
                summary_lines.push(format!("- {} ({})", description, cadence));
                watcher_records.push(data);
            }

            let object_refs = watcher_records
                .iter()
                .filter_map(|record| {
                    record
                        .get("watcher_id")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|id| serde_json::json!({ "kind": "watcher", "id": id }))
                })
                .collect::<Vec<_>>();
            let count = watcher_records.len();
            let completion_detail = format!("Saved {} watcher(s).", count);
            return Some(format!(
                "{}\n{}\n{}",
                render_tool_completion_marker_with_data(
                    "watch",
                    "completed",
                    &completion_detail,
                    serde_json::json!({
                        "watcher_count": count,
                        "watchers": watcher_records,
                        "object_refs": object_refs,
                    }),
                ),
                completion_detail,
                summary_lines.join("\n")
            ));
        }

        if let Err(message) = self
            .authorize_automation_tool_call("watch", arguments, authorization)
            .await
        {
            return Some(message);
        }
        let allow_duplicate = arguments
            .get("allow_duplicate")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let explicit_watcher_id = match arguments
            .get("watcher_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(value) => match uuid::Uuid::parse_str(value) {
                Ok(id) => Some(id),
                Err(error) => {
                    return Some(format!(
                        "Invalid watcher_id `{}`: {}. Use `list_watchers` and retry with the watcher ID.",
                        value, error
                    ));
                }
            },
            None => None,
        };
        let existing_watcher_target = match explicit_watcher_id {
            Some(id) => match self.watcher_manager.get(id).await {
                Some(watcher) => Some(watcher),
                None => {
                    return Some(format!(
                        "Watcher `{}` was not found. Use `list_watchers` and retry with an active watcher ID.",
                        id
                    ));
                }
            },
            None => None,
        };
        let description = arguments
            .get("description")
            .and_then(|v| v.as_str())
            .map(|value| value.to_string())
            .or_else(|| {
                existing_watcher_target
                    .as_ref()
                    .map(|watcher| watcher.description.clone())
            })
            .unwrap_or_else(|| "Background watcher".to_string());
        let script_poll_arguments = arguments
            .get("script")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|script| {
                let language = arguments
                    .get("script_language")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("python");
                serde_json::json!({
                    "language": language,
                    "code": script,
                    "network_access": arguments.get("network_access").and_then(|value| value.as_bool()).unwrap_or(false),
                    "execution_contract": {
                        "phase": "poll",
                        "target_validated_when_successful": true,
                        "ready_for_watch_when_successful": true
                    },
                    "context_from": arguments.get("context_from").cloned().unwrap_or_else(|| serde_json::json!([])),
                    "workdir": arguments.get("workdir").cloned().unwrap_or(serde_json::Value::Null),
                })
            });

        let Some(poll_action_value) = arguments
            .get("poll_action")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .or_else(|| {
                existing_watcher_target
                    .as_ref()
                    .map(|watcher| watcher.poll_action.clone())
                    .or_else(|| {
                        script_poll_arguments
                            .as_ref()
                            .map(|_| "code_execute".to_string())
                    })
            })
        else {
            return Some(
                    "Watcher requires `poll_action`/`poll_arguments` or a `script` so it knows what to poll. If the polling source is unclear, ask the user before retrying."
                        .to_string(),
                );
        };
        let mut poll_action = poll_action_value;
        let mut poll_arguments = arguments
            .get("poll_arguments")
            .cloned()
            .or_else(|| script_poll_arguments.clone())
            .or_else(|| {
                existing_watcher_target
                    .as_ref()
                    .map(|watcher| watcher.poll_arguments.clone())
            })
            .unwrap_or_else(|| serde_json::json!({}));
        let condition = if let Some(value) = arguments.get("condition") {
            match serde_json::from_value::<crate::core::watcher::WatchCondition>(value.clone()) {
                Ok(condition) => condition,
                Err(error) => {
                    return Some(format!(
                        "Watcher requires a valid structured `condition` object: {}",
                        error
                    ));
                }
            }
        } else if let Some(existing) = existing_watcher_target.as_ref() {
            existing.condition.clone()
        } else {
            return Some(
                "Watcher requires a structured `condition` object with `description` and `type`."
                    .to_string(),
            );
        };
        if let Err(error) = condition.validate() {
            return Some(format!("Watcher condition is invalid: {}", error));
        }
        let on_trigger = arguments
            .get("on_trigger")
            .and_then(|v| v.as_str())
            .map(|value| value.to_string())
            .or_else(|| {
                existing_watcher_target
                    .as_ref()
                    .map(|watcher| watcher.on_trigger.clone())
            })
            .unwrap_or_else(|| "Notify user with the result".to_string());
        let interval_secs = arguments
            .get("interval_secs")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                existing_watcher_target
                    .as_ref()
                    .map(|watcher| watcher.interval_secs)
            })
            .unwrap_or(60);
        let until_stopped = arguments
            .get("until_stopped")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let repeat_on_match = watcher_repeat_on_match_from_arguments(
            arguments,
            existing_watcher_target.as_ref(),
            until_stopped,
        );
        let timeout_hours = arguments.get("timeout_hours").and_then(|v| v.as_u64());
        let timeout_days = arguments.get("timeout_days").and_then(|v| v.as_u64());
        let requested_timeout_secs = arguments.get("timeout_secs").and_then(|v| v.as_u64());
        let timeout_secs = if until_stopped {
            super::watcher::MAX_TIMEOUT_SECS
        } else if let Some(days) = timeout_days {
            days.saturating_mul(24 * 60 * 60)
        } else if let Some(hours) = timeout_hours {
            hours.saturating_mul(60 * 60)
        } else if let Some(secs) = requested_timeout_secs {
            secs
        } else if let Some(existing) = existing_watcher_target.as_ref() {
            existing.timeout_secs
        } else {
            super::watcher::DEFAULT_TIMEOUT_SECS
        }
        .min(super::watcher::MAX_TIMEOUT_SECS);
        let notify_channel = normalize_automation_notification_channel(
            arguments
                .get("notify_channel")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    existing_watcher_target
                        .as_ref()
                        .map(|watcher| watcher.notify_channel.as_str())
                }),
        );
        let all_actions = self
            .runtime
            .list_enabled_actions()
            .await
            .unwrap_or_default();
        let condition_summary = condition.summary();
        let watcher_request_text = format!(
            "{}\nCondition: {}\nWhen triggered: {}",
            description.trim(),
            condition_summary,
            on_trigger.trim()
        );
        let validated_plan = self
            .validate_automation_plan(
                request_channel,
                AutomationSurface::Watch,
                &watcher_request_text,
                None,
                poll_action.clone(),
                poll_arguments.clone(),
                notify_channel.clone(),
                &all_actions,
            )
            .await;
        if let Some(reason) = validated_plan.blocked_reason {
            return Some(reason);
        }
        poll_action = validated_plan.action_name;
        poll_arguments = validated_plan.action_arguments;
        let notify_channel = validated_plan.delivery_channel;
        let planner_note = if validated_plan.notes.is_empty() {
            String::new()
        } else {
            format!(
                "\n\nPlanner note: {}",
                safe_truncate(&validated_plan.notes.join(" "), 220)
            )
        };
        poll_arguments = match self
            .normalize_action_arguments(&poll_action, &poll_arguments, &description)
            .await
        {
            Ok(normalized) => normalized,
            Err(error) => return Some(error),
        };
        let explicit_background_session_id =
            super::background_session::background_session_id_from_automation(&poll_arguments);
        let background_session = self
            .ensure_background_session_for_automation(
                request_channel,
                conversation_id,
                project_id,
                explicit_background_session_id.as_deref(),
                &description,
                "Keep the watcher active and report when its condition is met.",
                background_session_policy_for_action(&all_actions, &poll_action),
            )
            .await;

        let origin = AutomationOriginContext {
            channel: Some(request_channel.to_string()),
            conversation_id: conversation_id.map(|value| value.to_string()),
            project_id: project_id.map(|value| value.to_string()),
            source: Some("watcher".to_string()),
        };
        let default_validation = if Self::watch_condition_requires_previous_result(&condition) {
            AutomationValidation::default()
        } else {
            AutomationValidation {
                mode: AutomationValidationMode::NonEmptyResult,
                ..AutomationValidation::default()
            }
        };
        let policy = automation_policy_from_request_argument(
            arguments,
            AutomationExecutionPolicy {
                validation: automation_validation_from_request_argument(
                    arguments,
                    default_validation,
                ),
                stall_timeout_secs: timeout_secs.min(6 * 60 * 60),
                ..AutomationExecutionPolicy::default()
            },
        );
        poll_arguments = inject_automation_context(&poll_arguments, origin, policy.clone());
        poll_arguments = inject_automation_authorization_context(&poll_arguments, authorization);
        let background_session_id = background_session
            .as_ref()
            .map(|session| session.id.as_str())
            .or(explicit_background_session_id.as_deref());
        poll_arguments = super::background_session::set_background_session_id_in_automation(
            &poll_arguments,
            background_session_id,
        );
        if let Err(error) = self
            .enforce_background_session_policy_for_action(&poll_action, &poll_arguments)
            .await
        {
            return Some(error);
        }
        let watcher_auth = automation_runtime_authorization_context(
            &poll_arguments,
            ActionExecutionSurface::Background,
        );
        if let Err(error) = self
            .runtime
            .validate_action_invocation_with_context(&poll_action, &poll_arguments, &watcher_auth)
            .await
        {
            return Some(format!(
                "Watcher poll action `{}` is not runnable yet: {}. Use a poll action compatible with this target, or validate a custom poller with `code_execute` before creating the watcher.",
                poll_action, error
            ));
        }
        let mut initial_baseline_result: Option<String> = None;
        if poll_action.eq_ignore_ascii_case("code_execute")
            || Self::watch_condition_requires_structured_payload(&condition)
            || Self::watch_condition_requires_previous_result(&condition)
        {
            let preflight_timeout_secs = policy.stall_timeout_secs.clamp(30, 120);
            let preflight = tokio::time::timeout(
                std::time::Duration::from_secs(preflight_timeout_secs),
                self.runtime.execute_action_with_context(
                    &poll_action,
                    &poll_arguments,
                    &watcher_auth,
                ),
            )
            .await;
            let preflight_output = match preflight {
                Ok(Ok(output)) => output,
                Ok(Err(error)) => {
                    return Some(format!(
                        "Watcher poll action `{}` preflight failed: {}. The watcher was not created. Repair the poller so each poll returns a usable result even when the trigger condition is false.",
                        poll_action, error
                    ));
                }
                Err(_) => {
                    return Some(format!(
                        "Watcher poll action `{}` preflight timed out after {} seconds. The watcher was not created.",
                        poll_action, preflight_timeout_secs
                    ));
                }
            };
            let critique =
                critique_automation_result(&policy.validation, Some(&preflight_output), None);
            if !critique.validation_passed {
                return Some(format!(
                    "Watcher poll action `{}` preflight failed: {} The watcher was not created. Repair the poller so each poll returns a usable result even when the trigger condition is false.",
                    poll_action, critique.summary
                ));
            }
            if let Some(Err(error)) =
                Self::evaluate_watch_condition_without_llm(&condition, &preflight_output, None)
            {
                return Some(format!(
                    "Watcher condition is not compatible with the current poller output yet: {}. Repair the poller or the condition contract before creating the watcher.",
                    error
                ));
            }
            if Self::watch_condition_requires_previous_result(&condition) {
                initial_baseline_result = Some(preflight_output.clone());
            }
        }

        let created_at = chrono::Utc::now();
        let has_initial_baseline = initial_baseline_result.is_some();
        let baseline_last_poll_at = has_initial_baseline.then_some(created_at);
        let watcher = super::watcher::Watcher {
            id: uuid::Uuid::new_v4(),
            description: description.clone(),
            poll_action: poll_action.clone(),
            poll_arguments,
            condition,
            on_trigger: on_trigger.clone(),
            interval_secs,
            timeout_secs,
            notify_channel: notify_channel.clone(),
            repeat_on_match,
            status: super::watcher::WatcherStatus::Active,
            created_at,
            last_poll_at: baseline_last_poll_at,
            poll_count: if has_initial_baseline { 1 } else { 0 },
            trigger_result: None,
            last_result: initial_baseline_result,
            last_error: None,
            consecutive_failures: 0,
            next_poll_not_before: None,
            last_poll_outcome: has_initial_baseline
                .then_some(super::watcher::WatcherPollOutcome::NoMatch),
            notification_attempts: Vec::new(),
        };

        if explicit_watcher_id.is_none() && !allow_duplicate {
            let existing_watchers = self.watcher_manager.list().await;
            if let Some(prompt) =
                Self::watcher_update_confirmation_prompt(&watcher, &existing_watchers)
            {
                return Some(prompt);
            }
        }

        let (id, reused_existing, removed_duplicates) =
            if allow_duplicate && explicit_watcher_id.is_none() {
                (self.watcher_manager.add(watcher).await, false, 0)
            } else {
                match self
                    .watcher_manager
                    .upsert_similar(watcher, explicit_watcher_id)
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(message) => return Some(message),
                }
            };
        if let Some(saved_watcher) = self.watcher_manager.get(id).await {
            self.sync_watcher_supervisor_state(&saved_watcher, Some("active"), None)
                .await;
        }
        if let Some(cid) = conversation_id.filter(|value| !value.trim().is_empty()) {
            let watcher_id_text = id.to_string();
            let summary = if reused_existing {
                "Recently updated watcher in this conversation"
            } else {
                "Recently created watcher in this conversation"
            };
            self.persist_conversation_artifact_context(
                cid,
                ConversationArtifactSpec {
                    artifact_type: "watcher",
                    artifact_id: &watcher_id_text,
                    title: &description,
                    summary,
                    url: None,
                    related_actions: &["list_watchers", "watch"],
                },
            )
            .await;
        }
        if let Some(session_id) = background_session_id {
            self.attach_items_to_background_session(session_id, &[], &[id.to_string()])
                .await;
        }
        let display_notify_channel = if notify_channel == "preferred" {
            background_session
                .as_ref()
                .and_then(|session| session.preferred_delivery_channel.clone())
                .unwrap_or(notify_channel.clone())
        } else {
            notify_channel.clone()
        };

        // Human-readable duration
        let duration_desc = if until_stopped {
            "until you stop it".to_string()
        } else if timeout_secs >= 24 * 3600 {
            let days = timeout_secs / (24 * 3600);
            let hours = (timeout_secs % (24 * 3600)) / 3600;
            if hours > 0 {
                format!("{} day(s) {} hour(s)", days, hours)
            } else {
                format!("{} day(s)", days)
            }
        } else if timeout_secs >= 3600 {
            let hours = timeout_secs / 3600;
            let mins = (timeout_secs % 3600) / 60;
            if mins > 0 {
                format!("{} hour(s) {} min", hours, mins)
            } else {
                format!("{} hour(s)", hours)
            }
        } else {
            format!("{} minutes", timeout_secs / 60)
        };

        let user_specified_timeout =
            requested_timeout_secs.is_some() || timeout_hours.is_some() || timeout_days.is_some();
        let duration_note = if !user_specified_timeout {
            "\n\nThis watcher defaults to 24 hours. You can extend it later to any duration, including effectively until stopped."
        } else {
            ""
        };
        let delivery_note = if notify_channel == "preferred" {
            "\n\nExternal delivery is optional. If you later connect or remove notification channels, this watcher resolves the available channel at delivery time and otherwise keeps using in-app notifications."
        } else {
            ""
        };

        let action_sentence = if reused_existing {
            "Updated the existing background watcher."
        } else {
            "Created the background watcher."
        };
        let duplicate_note = if removed_duplicates > 0 {
            format!(
                "\n\nRemoved {} duplicate watcher(s) with the same purpose.",
                removed_duplicates
            )
        } else {
            String::new()
        };
        let cadence_desc = watcher_cadence_label(interval_secs);
        let delivery_desc = watcher_delivery_label(&display_notify_channel);
        let delivery_fragment = watcher_delivery_sentence_fragment(&display_notify_channel);
        let trigger_behavior_desc = if repeat_on_match {
            "keep polling and notify again on each match"
        } else {
            "stop after the first match"
        };
        let trigger_desc = on_trigger.trim().trim_end_matches(['.', '!', '?']);
        let watch_target = description.trim().trim_end_matches(['.', '!', '?']);
        let target_sentence = if watch_target.is_empty() {
            String::new()
        } else {
            format!(" Target: {}.", watch_target)
        };
        let target_block = if watch_target.is_empty() {
            String::new()
        } else {
            format!("\n\nTarget: {}", watch_target)
        };
        let completion_detail = format!(
            "{} It will check {} and notify {} when: {}. It will {}.{}",
            action_sentence,
            cadence_desc,
            delivery_fragment,
            if trigger_desc.is_empty() {
                "the condition is met"
            } else {
                trigger_desc
            },
            trigger_behavior_desc,
            target_sentence
        );

        Some(format!(
            "{}\n{}\n\n\
             It will check {} and notify {} when: {}.{}\n\n\
             - Cadence: {}\n\
             - Notifications: {}\n\
             - Trigger behavior: {}\n\
             - Duration: {}\n\n\
             You can stop or edit it from Background Work.{}{}{}{}",
            render_tool_completion_marker_with_data(
                "watch",
                "completed",
                &completion_detail,
                serde_json::json!({
                    "kind": "watcher",
                    "watcher_id": id.to_string(),
                    "background_session_id": background_session_id,
                    "description": description.clone(),
                    "cadence": cadence_desc.clone(),
                    "duration": duration_desc.clone(),
                    "notification": delivery_desc.clone(),
                    "repeat_on_match": repeat_on_match,
                    "trigger_behavior": trigger_behavior_desc,
                }),
            ),
            action_sentence,
            cadence_desc,
            delivery_fragment,
            if trigger_desc.is_empty() {
                "the condition is met"
            } else {
                trigger_desc
            },
            target_block,
            cadence_desc,
            delivery_desc,
            trigger_behavior_desc,
            duration_desc,
            duration_note,
            delivery_note,
            planner_note,
            duplicate_note
        ))
    }

    pub(crate) fn watcher_followup_worker(&self) -> WatcherFollowupWorker {
        WatcherFollowupWorker::from_agent(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(name: &str, description: &str, capabilities: &[&str]) -> crate::actions::ActionDef {
        crate::actions::ActionDef {
            name: name.to_string(),
            description: description.to_string(),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
            capabilities: capabilities.iter().map(|value| value.to_string()).collect(),
            ..crate::actions::ActionDef::default()
        }
    }

    fn test_task(action: &str, arguments: serde_json::Value) -> crate::core::task::Task {
        crate::core::task::Task {
            id: uuid::Uuid::new_v4(),
            description: "Scheduled test task".to_string(),
            action: action.to_string(),
            arguments,
            approval: crate::core::task::TaskApproval::Auto,
            capabilities: vec![action.to_string()],
            status: crate::core::task::TaskStatus::Pending,
            created_at: chrono::Utc::now(),
            scheduled_for: Some(chrono::Utc::now()),
            cron: None,
            result: None,
            proof_id: None,
            priority: None,
            urgency: None,
            importance: None,
            eisenhower_quadrant: None,
        }
    }

    #[test]
    fn scheduled_notify_user_tasks_execute_directly_and_self_deliver() {
        let notify = test_task(
            "notify_user",
            serde_json::json!({
                "message": "Reminder: meeting with Mark",
                "report_to": "telegram"
            }),
        );
        let web_search = test_task("web_search", serde_json::json!({"query": "status"}));

        assert!(scheduled_task_uses_direct_notify_user_execution(&notify));
        assert!(!scheduled_task_uses_direct_notify_user_execution(
            &web_search
        ));
        assert!(!scheduled_task_should_deliver_output_after_execution(
            &notify
        ));
        assert!(scheduled_task_should_deliver_output_after_execution(
            &web_search
        ));
    }

    #[test]
    fn scheduled_notify_user_execution_arguments_promote_report_to_route() {
        let task = test_task(
            "notify_user",
            serde_json::json!({
                "message": "Reminder: meeting with Mark",
                "report_to": "telegram"
            }),
        );

        let arguments = scheduled_notify_user_execution_arguments(&task);

        assert_eq!(
            arguments.get("message").and_then(|value| value.as_str()),
            Some("Reminder: meeting with Mark")
        );
        assert_eq!(
            arguments
                .get("delivery_channel")
                .and_then(|value| value.as_str()),
            Some("telegram")
        );
        assert_eq!(
            arguments.get("source").and_then(|value| value.as_str()),
            Some("reminder")
        );
        assert_eq!(
            arguments
                .get("in_app_title")
                .and_then(|value| value.as_str()),
            Some("Reminder")
        );
    }

    #[test]
    fn task_automation_run_record_can_exist_before_last_run_references() {
        let task = test_task("notify_user", serde_json::json!({"message": "Reminder"}));
        let started_at = chrono::DateTime::parse_from_rfc3339("2026-05-26T17:38:08Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let policy = AutomationExecutionPolicy::default();
        let critique = AutomationCritique {
            summary: "Execution started.".to_string(),
            retryable: false,
            validation_passed: false,
        };

        let record = task_automation_run_record(
            &task,
            "run-1",
            AutomationRunStatus::Running,
            1,
            started_at,
            None,
            AutomationOriginContext::default(),
            policy,
            critique,
            None,
            None,
            None,
        );

        assert_eq!(record.id, "run-1");
        assert_eq!(record.automation_id, task.id.to_string());
        assert_eq!(record.status, AutomationRunStatus::Running);
        assert_eq!(record.completed_at, None);
        assert_eq!(record.duration_ms, None);
    }

    #[test]
    fn scheduler_default_selection_never_invents_app_deploy() {
        let app_deploy = action(
            "app_deploy",
            "Deploy a browser application and return a live URL.",
            &["app_hosting"],
        );
        let read_only = action(
            "google_drive_search",
            "Search connected Google Drive files.",
            &["google_workspace"],
        );
        let notify = action("notify_user", "Notify the user.", &[]);

        assert!(!action_is_scheduler_default_candidate(&app_deploy));
        assert!(action_is_scheduler_default_candidate(&read_only));
        assert!(action_is_scheduler_default_candidate(&notify));
    }

    #[test]
    fn schedule_batch_items_inherit_and_override_delivery_route() {
        let args = serde_json::json!({
            "report_to": "preferred",
            "action": "notify_user",
            "action_arguments": {"message": "Reminder"},
            "items": [
                {
                    "task": "Meeting with Steve",
                    "at": "2026-06-30T03:30:00+05:30"
                },
                {
                    "task": "Private in-app reminder",
                    "at": "2026-09-30T03:30:00+05:30",
                    "report_to": "in_app"
                }
            ]
        });

        let items = schedule_task_batch_item_arguments(&args)
            .expect("batch should be detected")
            .expect("batch should validate");

        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].get("report_to").and_then(|v| v.as_str()),
            Some("preferred")
        );
        assert_eq!(
            items[1].get("report_to").and_then(|v| v.as_str()),
            Some("in_app")
        );
        assert_eq!(
            items[0].get("action").and_then(|v| v.as_str()),
            Some("notify_user")
        );
        assert_eq!(
            items[1].get("action").and_then(|v| v.as_str()),
            Some("notify_user")
        );
    }

    #[test]
    fn schedule_batch_accepts_structured_local_time_and_message_body() {
        let args = serde_json::json!({
            "report_to": "telegram",
            "action": "notify_user",
            "timezone": "Asia/Kolkata",
            "items": [
                {
                    "local_time": "00:22",
                    "action_arguments": {"message": "Meeting with Mark"}
                }
            ]
        });

        let items = schedule_task_batch_item_arguments(&args)
            .expect("batch should be detected")
            .expect("batch should validate");

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].get("local_time").and_then(|value| value.as_str()),
            Some("00:22")
        );
        assert_eq!(
            items[0]
                .get("action_arguments")
                .and_then(|value| value.get("message"))
                .and_then(|value| value.as_str()),
            Some("Meeting with Mark")
        );
    }

    #[test]
    fn schedule_task_requires_explicit_schedule() {
        let error = schedule_task_schedule_from_arguments(
            &serde_json::json!({
                "task": "Send reminder"
            }),
            ScheduleTaskScheduleContext::default(),
        )
        .expect_err("missing schedule should not default to now");

        assert!(error.contains("requires `cron`, `at`, `scheduled_for`, or `local_time`"));
        assert!(error.contains("refusing to infer the current time"));
    }

    #[test]
    fn schedule_task_validation_failures_are_structured_failed_completions() {
        let result = schedule_task_validation_failure_result(
            "Task scheduling requires `task` unless `task_id` points at an existing task.",
            "missing_task",
        );

        let completion = crate::runtime::parse_schedule_task_completion(&result)
            .expect("validation failure should use a structured schedule_task marker");
        assert_eq!(completion.tool, "schedule_task");
        assert_eq!(completion.status, "failed");
        assert!(result.contains("durable_commit"));
        assert!(result.contains("missing_task"));
    }

    #[test]
    fn schedule_task_description_can_use_notify_user_message() {
        let args = serde_json::json!({
            "at": "2026-05-22T13:06:00+05:30",
            "action": "notify_user",
            "action_arguments": {
                "message": "Meeting with Mark"
            },
            "report_to": "telegram"
        });

        assert_eq!(
            schedule_task_description_from_arguments(&args, None).as_deref(),
            Some("Meeting with Mark")
        );
    }

    #[test]
    fn schedule_task_parses_requested_absolute_time() {
        let (cron, at) = schedule_task_schedule_from_arguments(
            &serde_json::json!({
                "task": "Send reminder",
                "at": "2026-05-22T13:06:00+05:30"
            }),
            ScheduleTaskScheduleContext::default(),
        )
        .expect("valid absolute timestamp");

        assert!(cron.is_none());
        assert_eq!(at.unwrap().to_rfc3339(), "2026-05-22T07:36:00+00:00");
    }

    #[test]
    fn schedule_task_accepts_persisted_scheduled_for_as_absolute_time() {
        let (cron, at) = schedule_task_schedule_from_arguments(
            &serde_json::json!({
                "task": "Send reminder",
                "scheduled_for": "2026-05-22T13:06:00+05:30"
            }),
            ScheduleTaskScheduleContext::default(),
        )
        .expect("valid persisted timestamp");

        assert!(cron.is_none());
        assert_eq!(at.unwrap().to_rfc3339(), "2026-05-22T07:36:00+00:00");
    }

    #[test]
    fn schedule_task_resolves_time_only_update_against_existing_local_date() {
        let mut existing = test_task("notify_user", serde_json::json!({"message": "Reminder"}));
        existing.scheduled_for = Some(
            chrono::DateTime::parse_from_rfc3339("2026-05-27T23:08:00+05:30")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );
        let now_utc = chrono::DateTime::parse_from_rfc3339("2026-05-27T00:18:00+05:30")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let context = ScheduleTaskScheduleContext {
            now_utc,
            existing_task_target: Some(&existing),
            default_timezone: Some(chrono_tz::Asia::Kolkata),
        };

        let (cron, at) = schedule_task_schedule_from_arguments(
            &serde_json::json!({
                "task_id": existing.id.to_string(),
                "local_time": "12:22AM"
            }),
            context,
        )
        .expect("time-only update should resolve deterministically");

        assert!(cron.is_none());
        assert_eq!(at.unwrap().to_rfc3339(), "2026-05-26T18:52:00+00:00");
    }

    #[test]
    fn schedule_task_resolves_time_only_create_to_next_local_occurrence() {
        let now_utc = chrono::DateTime::parse_from_rfc3339("2026-05-27T00:18:00+05:30")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let context = ScheduleTaskScheduleContext {
            now_utc,
            existing_task_target: None,
            default_timezone: Some(chrono_tz::Asia::Kolkata),
        };

        let (cron, at) = schedule_task_schedule_from_arguments(
            &serde_json::json!({
                "task": "Send reminder",
                "local_time": "00:22"
            }),
            context,
        )
        .expect("time-only create should resolve to the next occurrence");

        assert!(cron.is_none());
        assert_eq!(at.unwrap().to_rfc3339(), "2026-05-26T18:52:00+00:00");
    }

    #[test]
    fn schedule_task_five_field_cron_is_expanded_without_current_time() {
        let (cron, at) = schedule_task_schedule_from_arguments(
            &serde_json::json!({
                "task": "Recurring reminder",
                "cron": "6 13 * * *"
            }),
            ScheduleTaskScheduleContext::default(),
        )
        .expect("valid cron");

        assert_eq!(cron.as_deref(), Some("0 6 13 * * *"));
        assert!(at.is_none());
    }
    #[test]
    fn watcher_batch_items_inherit_and_override_notification_route() {
        let args = serde_json::json!({
            "poll_action": "web_search",
            "poll_arguments": {"query": "provider pricing pages"},
            "condition": {
                "type": "material_change",
                "summary": "pricing or plan tiers changed"
            },
            "on_trigger": "Notify with the changed pricing details.",
            "interval_secs": 43200,
            "notify_channel": "preferred",
            "repeat_on_match": true,
            "items": [
                {
                    "description": "Monitor provider pricing"
                },
                {
                    "description": "Monitor internal dashboard status",
                    "notify_channel": "in_app"
                }
            ]
        });

        let items = watch_batch_item_arguments(&args)
            .expect("batch should be detected")
            .expect("batch should validate");

        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].get("notify_channel").and_then(|v| v.as_str()),
            Some("preferred")
        );
        assert_eq!(
            items[1].get("notify_channel").and_then(|v| v.as_str()),
            Some("in_app")
        );
        assert_eq!(
            items[0].get("interval_secs").and_then(|v| v.as_i64()),
            Some(43200)
        );
        assert_eq!(
            items[1].get("repeat_on_match").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            items[1].get("poll_action").and_then(|v| v.as_str()),
            Some("web_search")
        );
    }

    #[test]
    fn watcher_repeat_mode_uses_structured_contract() {
        let descriptive_args = serde_json::json!({
            "description": "monitor a source continuously and alert on later matches"
        });
        assert!(!watcher_repeat_on_match_from_arguments(
            &descriptive_args,
            None,
            false
        ));

        let explicit_args = serde_json::json!({ "repeat_on_match": true });
        assert!(watcher_repeat_on_match_from_arguments(
            &explicit_args,
            None,
            false
        ));

        let explicit_false_args = serde_json::json!({ "repeat_on_match": false });
        assert!(!watcher_repeat_on_match_from_arguments(
            &explicit_false_args,
            None,
            true
        ));

        assert!(watcher_repeat_on_match_from_arguments(
            &serde_json::json!({}),
            None,
            true
        ));
    }

    #[test]
    fn watcher_change_mode_requires_baseline_and_detects_difference() {
        let condition = crate::core::watcher::WatchCondition {
            description: "Trigger when the observed result changes".to_string(),
            evaluation_mode: crate::core::watcher::WatchConditionEvaluationMode::Change,
            matcher: crate::core::watcher::WatchConditionMatcher::NotEmpty,
        };

        assert_eq!(
            Agent::evaluate_watch_condition_without_llm(&condition, "alpha", None),
            Some(Ok(false))
        );
        assert_eq!(
            Agent::evaluate_watch_condition_without_llm(&condition, "alpha", Some("alpha")),
            Some(Ok(false))
        );
        assert_eq!(
            Agent::evaluate_watch_condition_without_llm(&condition, "beta", Some("alpha")),
            Some(Ok(true))
        );
    }

    #[test]
    fn chat_owned_task_reports_are_suppressed() {
        let task = crate::core::Task::new(
            "Deep research: market structure".to_string(),
            "chat_request".to_string(),
            serde_json::json!({
                "_origin": "chat",
                "_task_kind": "chat_request",
                "_work_type": "research",
                "deep_research": true,
                "message": "research market structure"
            }),
        );

        assert!(Agent::task_report_is_chat_owned(&task));
    }

    #[test]
    fn scheduled_research_task_reports_are_not_suppressed() {
        let task = crate::core::Task::new(
            "Scheduled market research".to_string(),
            "research".to_string(),
            serde_json::json!({
                "deep_research": true,
                "query": "research market structure",
                "report_to": "preferred"
            }),
        );

        assert!(!Agent::task_report_is_chat_owned(&task));
    }
}
