#![allow(dead_code)]

use super::*;

fn automation_delivery_channel_requires_connection(channel: &str) -> bool {
    let normalized = channel.trim().to_ascii_lowercase();
    !normalized.is_empty()
        && normalized != "preferred"
        && normalized != AUTOMATION_IN_APP_NOTIFICATION_CHANNEL
        && is_external_notification_channel(&normalized)
}

fn automation_unavailable_delivery_note(channel: &str) -> String {
    let display = notification_channel_display_name(channel);
    format!(
        "{} delivery is requested but not connected yet. AgentArk will save the automation with that requested route, keep updates in app while it is unavailable, and use {} automatically once the channel is connected.",
        display, display
    )
}

#[derive(Debug, Clone)]
pub(super) enum PendingConversationActionKind {
    ForceImportSkill,
    ResumeResilienceFollowup,
}

impl PendingConversationActionKind {
    pub(super) fn as_pending_action_kind(&self) -> &'static str {
        match self {
            Self::ForceImportSkill => "force_import_skill",
            Self::ResumeResilienceFollowup => "resume_resilience_followup",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct PendingConversationAction {
    pub(super) key: String,
    pub(super) summary: String,
    pub(super) kind: PendingConversationActionKind,
}

impl Agent {
    /// Get the data directory path
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Get the last user activity timestamp (for idle detection)
    pub fn last_activity_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.last_activity.try_read().ok().and_then(|guard| *guard)
    }

    pub fn active_message_request_count(&self) -> usize {
        self.active_message_requests.load(Ordering::Acquire)
    }

    pub(super) fn track_active_message_request(&self) -> ActiveMessageRequestGuard {
        self.active_message_requests.fetch_add(1, Ordering::AcqRel);
        ActiveMessageRequestGuard {
            counter: Arc::clone(&self.active_message_requests),
        }
    }

    /// Generate a short deterministic title from the user's first message.
    ///
    /// This intentionally avoids a second LLM call after the main response has already
    /// completed, so conversation metadata never blocks the live request path.
    pub(super) fn generate_conversation_title(&self, user_message: &str) -> String {
        let single_line = user_message
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default();
        let collapsed = single_line.split_whitespace().collect::<Vec<_>>().join(" ");
        let cleaned = collapsed
            .trim_matches(|ch: char| ch.is_ascii_punctuation() || ch.is_whitespace())
            .to_string();
        let candidate = if cleaned.is_empty() {
            user_message
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            cleaned
        };
        safe_truncate(&candidate, 48)
    }

    pub(super) async fn conversation_scope_mode(&self) -> ConversationScope {
        let raw = self
            .storage
            .get("conversation_scope_mode")
            .await
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok());
        ConversationScope::from_storage(raw.as_deref())
    }

    pub(super) fn retain_actions_for_connected_integrations(
        actions: &mut Vec<crate::actions::ActionDef>,
        calendar_available: bool,
        gmail_available: bool,
        google_workspace_granted_bundles: &[String],
    ) {
        let workspace_bundle_available = |bundle: &str| {
            google_workspace_granted_bundles
                .iter()
                .any(|granted| granted == bundle)
        };
        let workspace_any_bundle_available = !google_workspace_granted_bundles.is_empty();
        if !calendar_available {
            actions.retain(|action| {
                !matches!(
                    action.name.as_str(),
                    "calendar_today" | "calendar_list" | "calendar_create" | "calendar_free"
                )
            });
        }
        actions.retain(|action| match action.name.as_str() {
            "gmail_scan" | "gmail_reply" => gmail_available,
            "google_drive_search" => workspace_bundle_available("drive"),
            "google_docs_read" => workspace_bundle_available("docs"),
            "google_sheets_read" => workspace_bundle_available("sheets"),
            "google_chat_list_spaces" => workspace_bundle_available("chat"),
            "google_admin_list_users" => workspace_bundle_available("admin"),
            "google_workspace_gws_help"
            | "google_workspace_gws_schema"
            | "google_workspace_gws_skills"
            | "google_workspace_gws_command" => workspace_any_bundle_available,
            _ => true,
        });
    }

    pub(super) fn automation_candidate_overlap_score(request: &str, candidate_text: &str) -> usize {
        let query_tokens = Self::background_session_reference_tokens(request);
        if query_tokens.is_empty() {
            return 0;
        }
        let candidate_tokens = Self::background_session_reference_tokens(candidate_text);
        if candidate_tokens.is_empty() {
            return 0;
        }
        let overlap = query_tokens
            .iter()
            .filter(|token| candidate_tokens.contains(*token))
            .count();
        if overlap < 2 {
            return 0;
        }
        let required_overlap = if query_tokens.len() <= 4 {
            2
        } else {
            query_tokens.len().div_ceil(3).max(2)
        };
        if overlap < required_overlap {
            0
        } else {
            overlap
        }
    }

    pub(super) fn collect_automation_argument_reference_text(
        value: &serde_json::Value,
        out: &mut Vec<String>,
    ) {
        if out.len() >= 12 {
            return;
        }
        match value {
            serde_json::Value::String(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    out.push(safe_truncate(trimmed, 220));
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    Self::collect_automation_argument_reference_text(item, out);
                    if out.len() >= 12 {
                        break;
                    }
                }
            }
            serde_json::Value::Object(map) => {
                for (key, inner) in map {
                    if key.starts_with('_') || is_sensitive_tool_call_argument_key(key) {
                        continue;
                    }
                    Self::collect_automation_argument_reference_text(inner, out);
                    if out.len() >= 12 {
                        break;
                    }
                }
            }
            serde_json::Value::Number(_) | serde_json::Value::Bool(_) | serde_json::Value::Null => {
            }
        }
    }

    pub(super) fn automation_argument_reference_text(arguments: &serde_json::Value) -> Vec<String> {
        let mut out = Vec::new();
        Self::collect_automation_argument_reference_text(arguments, &mut out);
        out
    }

    pub(super) fn task_reference_text(task: &super::task::Task) -> String {
        let mut parts = vec![task.description.clone()];
        parts.extend(Self::automation_argument_reference_text(&task.arguments));
        parts.join(" ")
    }

    pub(super) fn watcher_reference_text(watcher: &super::watcher::Watcher) -> String {
        let mut parts = vec![
            watcher.description.clone(),
            watcher.condition.summary(),
            watcher.on_trigger.clone(),
        ];
        parts.extend(Self::automation_argument_reference_text(
            &watcher.poll_arguments,
        ));
        parts.join(" ")
    }

    pub(super) fn task_update_confirmation_prompt(
        candidate: &super::task::Task,
        existing_tasks: &[super::task::Task],
    ) -> Option<String> {
        if candidate.action.eq_ignore_ascii_case("notify_user")
            && (candidate.scheduled_for.is_some()
                || candidate
                    .cron
                    .as_deref()
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false))
        {
            return None;
        }
        if existing_tasks.iter().any(|existing| {
            !matches!(existing.status, super::task::TaskStatus::Completed)
                && super::task::tasks_are_semantically_similar(existing, candidate)
        }) {
            return None;
        }

        let request_text = Self::task_reference_text(candidate);
        let mut scored = existing_tasks
            .iter()
            .filter(|task| !matches!(task.status, super::task::TaskStatus::Completed))
            .filter_map(|task| {
                let mut score = Self::automation_candidate_overlap_score(
                    &request_text,
                    &Self::task_reference_text(task),
                );
                if task.action.eq_ignore_ascii_case(&candidate.action) && score > 0 {
                    score += 2;
                }
                (score > 0).then_some((score, task))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.created_at.cmp(&left.1.created_at))
        });
        if scored.is_empty() {
            return None;
        }

        let mut lines = vec![
            "Confirmation needed: I found existing tasks that might be related, but none is a close enough match to update automatically. Confirm which one to update, or say `create new`.".to_string(),
        ];
        for (_, task) in scored.iter().take(5) {
            lines.push(format!(
                "- {} (`{}`) [{}]",
                safe_truncate(&task.description, 120),
                task.id.to_string().chars().take(8).collect::<String>(),
                Self::task_status_debug_label(&task.status)
            ));
        }
        Some(lines.join("\n"))
    }

    pub(super) fn watcher_update_confirmation_prompt(
        candidate: &super::watcher::Watcher,
        existing_watchers: &[super::watcher::Watcher],
    ) -> Option<String> {
        if existing_watchers.iter().any(|watcher| {
            super::watcher::WatcherManager::watchers_are_semantically_similar(watcher, candidate)
        }) {
            return None;
        }

        let request_text = Self::watcher_reference_text(candidate);
        let mut scored = existing_watchers
            .iter()
            .filter_map(|watcher| {
                let mut score = Self::automation_candidate_overlap_score(
                    &request_text,
                    &Self::watcher_reference_text(watcher),
                );
                if watcher
                    .poll_action
                    .eq_ignore_ascii_case(candidate.poll_action.as_str())
                    && score > 0
                {
                    score += 2;
                }
                (score > 0).then_some((score, watcher))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.created_at.cmp(&left.1.created_at))
        });
        if scored.is_empty() {
            return None;
        }

        let mut lines = vec![
            "Confirmation needed: I found existing watchers that might be related, but none is a close enough match to update automatically. Confirm which one to update, or say `create new`.".to_string(),
        ];
        for (_, watcher) in scored.iter().take(5) {
            lines.push(format!(
                "- {} (`{}`) [{}] every {}s",
                safe_truncate(&watcher.description, 120),
                watcher.id.to_string().chars().take(8).collect::<String>(),
                Self::watcher_supervisor_status_label(&watcher.status),
                watcher.interval_secs
            ));
        }
        Some(lines.join("\n"))
    }

    pub(super) fn action_action_metadata_for_name(
        all_actions: &[crate::actions::ActionDef],
        action_name: &str,
    ) -> crate::actions::ActionMetadata {
        if let Some(action) = all_actions
            .iter()
            .find(|candidate| candidate.name == action_name)
        {
            return action.action_metadata();
        }

        crate::actions::action_metadata_for_action(&crate::actions::ActionDef {
            name: action_name.to_string(),
            ..crate::actions::ActionDef::default()
        })
    }

    pub(super) fn heuristic_automation_intent_assessment(
        surface: AutomationSurface,
        _request_text: &str,
        _action_name: &str,
        _delivery_channel: &str,
        trigger_kind_hint: Option<&str>,
        action_meta: &crate::actions::ActionMetadata,
    ) -> AutomationIntentAssessment {
        let trigger_kind = trigger_kind_hint
            .map(str::trim)
            .filter(|value| {
                matches!(
                    *value,
                    "absolute_date"
                        | "relative_time"
                        | "recurring_schedule"
                        | "poll_until"
                        | "external_state"
                        | "unknown"
                )
            })
            .unwrap_or(match surface {
                AutomationSurface::Schedule => "recurring_schedule",
                AutomationSurface::Watch => "external_state",
            });
        let integration_class =
            action_integration_class_name(&action_meta.integration_class).to_string();

        AutomationIntentAssessment {
            trigger_kind: trigger_kind.to_string(),
            delivery_policy: "preferred_single_channel".to_string(),
            source_policy: "existing_action".to_string(),
            fanout: false,
            allowed_integration_classes: vec![integration_class],
            avoid_integration_classes: Vec::new(),
            reasoning: format!(
                "{} automation policy used selected action metadata and supplied automation arguments.",
                surface.as_str()
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn validate_automation_plan(
        &self,
        _channel: &str,
        surface: AutomationSurface,
        request_text: &str,
        trigger_kind_hint: Option<&str>,
        action_name: String,
        action_arguments: serde_json::Value,
        delivery_channel: String,
        all_actions: &[crate::actions::ActionDef],
    ) -> AutomationPlanValidationResult {
        let action_meta = Self::action_action_metadata_for_name(all_actions, &action_name);
        let assessment = Self::heuristic_automation_intent_assessment(
            surface,
            request_text,
            &action_name,
            &delivery_channel,
            trigger_kind_hint,
            &action_meta,
        );
        let mut action_name = action_name;
        let mut action_arguments = action_arguments;
        let mut delivery_channel = delivery_channel.trim().to_ascii_lowercase();
        let mut notes = Vec::new();

        if assessment.source_policy == "none" {
            return AutomationPlanValidationResult {
                action_name,
                action_arguments,
                delivery_channel,
                notes: vec![assessment.reasoning],
                blocked_reason: Some(
                    "Automation policy did not return a usable source policy, so no automation was scheduled.".to_string(),
                ),
            };
        }

        if assessment.delivery_policy == "in_app_only"
            && delivery_channel != AUTOMATION_IN_APP_NOTIFICATION_CHANNEL
        {
            delivery_channel = AUTOMATION_IN_APP_NOTIFICATION_CHANNEL.to_string();
            notes.push(
                "Planner policy: delivery stays in-app only because no external notification channel was requested."
                    .to_string(),
            );
        } else if assessment.fanout {
            notes.push(
                "Planner policy: reminder delivery uses one channel at a time, so this will stay single-route."
                    .to_string(),
            );
            if delivery_channel.is_empty() {
                delivery_channel = "preferred".to_string();
            }
        }

        if !delivery_channel.is_empty()
            && delivery_channel != "preferred"
            && delivery_channel != AUTOMATION_IN_APP_NOTIFICATION_CHANNEL
            && !is_external_notification_channel(&delivery_channel)
        {
            delivery_channel = "preferred".to_string();
            notes.push(
                "Planner policy: reminder delivery uses external notification channels or in-app, not workspace mail/calendar routes."
                    .to_string(),
            );
        }
        if automation_delivery_channel_requires_connection(&delivery_channel)
            && !self
                .notification_channel_is_configured_any(&delivery_channel)
                .await
        {
            notes.push(automation_unavailable_delivery_note(&delivery_channel));
        } else if delivery_channel == "preferred"
            && self.configured_push_channels().await.is_empty()
        {
            notes.push(
                "The automation is saved, but no messaging channel is connected yet. Push updates will not work until you connect Telegram, WhatsApp, Slack, or another channel in Settings > Channels. Until then updates will stay in-app."
                    .to_string(),
            );
        }

        let current_meta = Self::action_action_metadata_for_name(all_actions, &action_name);
        let current_class = action_integration_class_name(&current_meta.integration_class);
        let avoids_workspace = assessment
            .avoid_integration_classes
            .iter()
            .any(|value| value == "workspace");
        let reminder_like = matches!(
            assessment.trigger_kind.as_str(),
            "absolute_date" | "relative_time" | "recurring_schedule"
        ) && assessment.source_policy == "internal_first"
            && assessment
                .avoid_integration_classes
                .iter()
                .any(|value| value == "workspace");

        if matches!(surface, AutomationSurface::Watch)
            && matches!(
                assessment.trigger_kind.as_str(),
                "absolute_date" | "relative_time"
            )
            && assessment.source_policy == "internal_first"
            && current_class != "internal"
        {
            let timezone = {
                let profile = self.user_profile.read().await;
                profile
                    .timezone
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
            };
            action_name = "current_time".to_string();
            action_arguments =
                build_current_time_action_arguments(&action_arguments, timezone.as_deref());
            notes.push(
                "Planner policy: this watcher only needs an internal time trigger, so it was moved off workspace polling."
                    .to_string(),
            );
        }

        if matches!(surface, AutomationSurface::Schedule)
            && reminder_like
            && !schedule_action_is_internal_notification(&action_name)
            && (avoids_workspace
                || matches!(
                    current_class,
                    "workspace" | "search" | "browser" | "unknown" | "internal"
                ))
        {
            action_name = "notify_user".to_string();
            action_arguments = build_notify_user_action_arguments(
                &action_arguments,
                request_text,
                &delivery_channel,
            );
            notes.push(
                "Planner policy: this schedule is a reminder, so it will fire an internal notification instead of calling an external tool."
                    .to_string(),
            );
        } else if action_name == "notify_user" {
            action_arguments = build_notify_user_action_arguments(
                &action_arguments,
                request_text,
                &delivery_channel,
            );
        }

        if matches!(surface, AutomationSurface::Schedule) && action_name != "notify_user" {
            if let Some(payload) = action_arguments.as_object_mut() {
                if delivery_channel.is_empty() {
                    payload.remove("report_to");
                } else {
                    payload.insert(
                        "report_to".to_string(),
                        serde_json::Value::String(delivery_channel.clone()),
                    );
                }
            }
        }

        AutomationPlanValidationResult {
            action_name,
            action_arguments,
            delivery_channel,
            notes,
            blocked_reason: None,
        }
    }

    pub(crate) fn onboarding_profile_ready(
        profile: &UserProfile,
        preferred_name: Option<&str>,
        priority_focus: Option<&str>,
    ) -> bool {
        let timezone_ready = profile
            .timezone
            .as_ref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        let tone_ready = profile
            .tone
            .as_ref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        let name_ready = preferred_name
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        let priority_ready = priority_focus
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        timezone_ready && tone_ready && name_ready && priority_ready
    }

    pub(super) async fn recent_messages_for_intent_gating(
        &self,
        conversation_id: &str,
        _current_message: &str,
    ) -> Vec<ConversationMessage> {
        let history = {
            let guard = self.conversation_history.read().await;
            guard.get(conversation_id).cloned().unwrap_or_default()
        };
        if !history.is_empty() {
            return history;
        }

        let stored = self
            .encrypted_storage
            .get_recent_messages_decrypted(conversation_id, 8)
            .await
            .unwrap_or_default();
        stored
            .into_iter()
            .map(|msg| ConversationMessage {
                role: msg.role,
                content: msg.content,
                _timestamp: Self::parse_message_timestamp(&msg.timestamp),
            })
            .collect()
    }

    pub(super) async fn recent_trusted_assistant_message_for_inbound_guard(
        &self,
        conversation_id: &str,
        current_message: &str,
    ) -> Option<String> {
        if conversation_id.trim().is_empty() {
            return None;
        }

        self.recent_messages_for_intent_gating(conversation_id, current_message)
            .await
            .into_iter()
            .rev()
            .find(|message| message.role == "assistant")
            .map(|message| crate::security::normalize_for_analysis(&message.content))
            .map(|message| safe_truncate(&message, 600))
            .filter(|message| !message.trim().is_empty())
    }

    pub(super) async fn pending_conversation_actions(
        &self,
        conversation_id: &str,
    ) -> Vec<PendingConversationAction> {
        let mut out = Vec::new();
        if let Some(pending_import) = self.peek_pending_skill_import(conversation_id).await {
            out.push(PendingConversationAction {
                key: "skill_import_force".to_string(),
                summary: format!(
                    "Force-import the previously blocked skill '{}' from {} despite the earlier security warning.",
                    pending_import.skill_name, pending_import.source_url
                ),
                kind: PendingConversationActionKind::ForceImportSkill,
            });
        }
        if let Some(pending_followup) = self.load_pending_resilience_followup(conversation_id).await
        {
            if let Some(summary) = Self::pending_resilience_followup_summary(&pending_followup) {
                out.push(PendingConversationAction {
                    key: "resume_resilience_followup".to_string(),
                    summary,
                    kind: PendingConversationActionKind::ResumeResilienceFollowup,
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_external_delivery_route_is_preserved_for_later_setup() {
        assert!(automation_delivery_channel_requires_connection("telegram"));
        assert!(automation_delivery_channel_requires_connection("pagerduty"));
        assert!(!automation_delivery_channel_requires_connection(
            "preferred"
        ));
        assert!(!automation_delivery_channel_requires_connection(
            AUTOMATION_IN_APP_NOTIFICATION_CHANNEL
        ));

        let note = automation_unavailable_delivery_note("telegram");
        assert!(note.contains("Telegram delivery is requested"));
        assert!(note.contains("use Telegram automatically once the channel is connected"));

        let custom_note = automation_unavailable_delivery_note("pagerduty");
        assert!(custom_note.contains("pagerduty delivery is requested"));
        assert!(custom_note.contains("use pagerduty automatically once the channel is connected"));
    }

    fn test_action(name: &str) -> crate::actions::ActionDef {
        crate::actions::ActionDef {
            name: name.to_string(),
            ..crate::actions::ActionDef::default()
        }
    }

    #[test]
    fn disconnected_google_workspace_actions_are_not_exposed_to_chat_catalog() {
        let mut actions = vec![
            test_action("file_read"),
            test_action("gmail_scan"),
            test_action("calendar_today"),
            test_action("google_drive_search"),
            test_action("google_workspace_gws_command"),
        ];

        Agent::retain_actions_for_connected_integrations(&mut actions, false, false, &[]);

        let names = actions
            .iter()
            .map(|action| action.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["file_read"]);
    }

    #[test]
    fn granted_google_workspace_bundles_expose_only_matching_chat_actions() {
        let mut actions = vec![
            test_action("gmail_scan"),
            test_action("calendar_today"),
            test_action("google_drive_search"),
            test_action("google_docs_read"),
            test_action("google_workspace_gws_command"),
        ];

        Agent::retain_actions_for_connected_integrations(
            &mut actions,
            true,
            true,
            &["drive".to_string()],
        );

        let names = actions
            .iter()
            .map(|action| action.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "gmail_scan",
                "calendar_today",
                "google_drive_search",
                "google_workspace_gws_command",
            ]
        );
    }
}
