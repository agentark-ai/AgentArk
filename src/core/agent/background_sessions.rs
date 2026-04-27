use super::*;
use crate::core::{background_session, task, watcher};

#[derive(Debug, Clone)]
pub(super) struct BackgroundSessionResolution {
    pub(super) session: Option<background_session::BackgroundSession>,
    pub(super) ambiguous: bool,
    pub(super) candidates: Vec<background_session::BackgroundSession>,
}

pub(super) fn background_session_policy_for_action(
    all_actions: &[crate::actions::ActionDef],
    action_name: &str,
) -> background_session::BackgroundSessionPolicy {
    let meta = all_actions
        .iter()
        .find(|action| action.name.eq_ignore_ascii_case(action_name.trim()))
        .map(crate::actions::ActionDef::planner_metadata)
        .unwrap_or_default();
    background_session::BackgroundSessionPolicy {
        allowed_action_roles: vec![planner_action_role_name(&meta.role).to_string()],
        allowed_integration_classes: vec![
            planner_integration_class_name(&meta.integration_class).to_string(),
        ],
    }
    .normalized()
}

impl Agent {
    pub(super) fn background_session_artifact_context(
        session: &background_session::BackgroundSession,
    ) -> ConversationArtifactContext {
        ConversationArtifactContext {
            artifact_type: "background_session".to_string(),
            artifact_id: session.id.clone(),
            title: session.title.clone(),
            summary: session
                .summary
                .clone()
                .unwrap_or_else(|| safe_truncate(&session.objective, 220)),
            url: String::new(),
            related_actions: vec![
                "schedule_task".to_string(),
                "watch".to_string(),
                "notify_user".to_string(),
                "list_tasks".to_string(),
                "list_watchers".to_string(),
            ],
            updated_at: session.updated_at.to_rfc3339(),
        }
    }

    pub(super) async fn background_sessions_for_conversation(
        &self,
        conversation_id: &str,
        include_closed: bool,
    ) -> Vec<background_session::BackgroundSession> {
        self.background_sessions
            .list_for_conversation(conversation_id, include_closed)
            .await
    }

    fn explicit_background_session_reference<'a>(
        message: &str,
        sessions: &'a [background_session::BackgroundSession],
    ) -> Vec<&'a background_session::BackgroundSession> {
        let lowered = message.trim().to_ascii_lowercase();
        if lowered.is_empty() {
            return Vec::new();
        }
        sessions
            .iter()
            .filter(|session| {
                let session_id = session.id.to_ascii_lowercase();
                let short_id = session_id.chars().take(8).collect::<String>();
                lowered.contains(session_id.as_str()) || lowered.contains(short_id.as_str())
            })
            .collect()
    }

    pub(super) fn background_session_reference_tokens(value: &str) -> HashSet<String> {
        tokenize_lower(value)
            .into_iter()
            .filter(|token| token.len() >= 4 || token.chars().any(|ch| ch.is_ascii_digit()))
            .collect()
    }

    fn background_session_reference_text(
        session: &background_session::BackgroundSession,
    ) -> String {
        [
            Some(session.title.as_str()),
            Some(session.objective.as_str()),
            session.summary.as_deref(),
            session.current_focus.as_deref(),
            session.working_memory.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
    }

    fn background_session_candidate_score(
        message: &str,
        session: &background_session::BackgroundSession,
        recent_background_session_ids: &HashSet<String>,
    ) -> usize {
        if session.status.is_closed() {
            return 0;
        }
        let query_tokens = Self::background_session_reference_tokens(message);
        if query_tokens.is_empty() {
            return 0;
        }
        let session_tokens = Self::background_session_reference_tokens(
            &Self::background_session_reference_text(session),
        );
        if session_tokens.is_empty() {
            return 0;
        }
        let overlap = query_tokens
            .iter()
            .filter(|token| session_tokens.contains(*token))
            .count();
        let required_overlap = if recent_background_session_ids.contains(&session.id) {
            2
        } else {
            3
        };
        let required_coverage = if query_tokens.len() <= 3 {
            query_tokens.len()
        } else {
            (query_tokens.len() + 1) / 2
        };
        if overlap < required_overlap || overlap < required_coverage {
            return 0;
        }

        let mut score = overlap * 10;
        if recent_background_session_ids.contains(&session.id) {
            score += 6;
        }
        if !session.linked_task_ids.is_empty() || !session.linked_watcher_ids.is_empty() {
            score += 4;
        }
        score
    }

    pub(super) fn resolve_background_session_reference_from_candidates(
        message: &str,
        recent_artifacts: &[ConversationArtifactContext],
        sessions: &[background_session::BackgroundSession],
    ) -> BackgroundSessionResolution {
        if sessions.is_empty() {
            return BackgroundSessionResolution {
                session: None,
                ambiguous: false,
                candidates: Vec::new(),
            };
        }

        let explicit = Self::explicit_background_session_reference(message, sessions);
        if explicit.len() == 1 {
            return BackgroundSessionResolution {
                session: explicit.first().cloned().cloned(),
                ambiguous: false,
                candidates: sessions.to_vec(),
            };
        }
        if explicit.len() > 1 {
            return BackgroundSessionResolution {
                session: None,
                ambiguous: true,
                candidates: explicit.into_iter().cloned().collect(),
            };
        }

        let recent_background_session_ids = recent_artifacts
            .iter()
            .filter(|artifact| {
                artifact
                    .artifact_type
                    .trim()
                    .eq_ignore_ascii_case("background_session")
            })
            .map(|artifact| artifact.artifact_id.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<HashSet<_>>();

        let mut scored = sessions
            .iter()
            .filter_map(|session| {
                let score = Self::background_session_candidate_score(
                    message,
                    session,
                    &recent_background_session_ids,
                );
                (score > 0).then_some((score, session))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.updated_at.cmp(&left.1.updated_at))
        });

        if let Some((best_score, best_session)) = scored.first().copied() {
            let second_score = scored.get(1).map(|(score, _)| *score).unwrap_or(0);
            if second_score == 0 || best_score >= second_score + 3 {
                return BackgroundSessionResolution {
                    session: Some((*best_session).clone()),
                    ambiguous: false,
                    candidates: sessions.to_vec(),
                };
            }
            return BackgroundSessionResolution {
                session: None,
                ambiguous: true,
                candidates: scored
                    .into_iter()
                    .filter(|(score, _)| *score == best_score)
                    .map(|(_, session)| (*session).clone())
                    .collect(),
            };
        }

        BackgroundSessionResolution {
            session: None,
            ambiguous: false,
            candidates: sessions.to_vec(),
        }
    }

    pub(super) async fn resolve_background_session_reference(
        &self,
        conversation_id: &str,
        message: &str,
        recent_artifacts: &[ConversationArtifactContext],
        include_closed: bool,
    ) -> BackgroundSessionResolution {
        let sessions = self
            .background_sessions_for_conversation(conversation_id, include_closed)
            .await;
        let local_resolution = Self::resolve_background_session_reference_from_candidates(
            message,
            recent_artifacts,
            &sessions,
        );
        if local_resolution.session.is_some() || local_resolution.ambiguous {
            return local_resolution;
        }

        let mut all_sessions = self
            .background_sessions
            .list()
            .await
            .into_iter()
            .filter(|session| include_closed || !session.status.is_closed())
            .collect::<Vec<_>>();
        all_sessions.retain(|session| !sessions.iter().any(|local| local.id == session.id));
        if all_sessions.is_empty() {
            return local_resolution;
        }

        let global_resolution = Self::resolve_background_session_reference_from_candidates(
            message,
            recent_artifacts,
            &all_sessions,
        );
        if global_resolution.session.is_some()
            || global_resolution.ambiguous
            || local_resolution.candidates.is_empty()
        {
            global_resolution
        } else {
            local_resolution
        }
    }

    pub(super) async fn sync_background_session_artifact_context(
        &self,
        conversation_id: &str,
        session: &background_session::BackgroundSession,
    ) {
        let cid = conversation_id.trim();
        if cid.is_empty() {
            return;
        }
        self.persist_conversation_artifact_context_payload(
            cid,
            Self::background_session_artifact_context(session),
        )
        .await;
    }

    pub(super) async fn seed_background_session_policy_if_unset(
        &self,
        session: background_session::BackgroundSession,
        policy: &background_session::BackgroundSessionPolicy,
    ) -> background_session::BackgroundSession {
        if !session.policy.is_unset() || policy.is_unset() {
            return session;
        }
        self.background_sessions
            .update(
                &session.id,
                background_session::BackgroundSessionUpdate {
                    policy: Some(policy.clone()),
                    ..Default::default()
                },
                Some("agent"),
            )
            .await
            .unwrap_or(session)
    }

    pub(super) async fn ensure_background_session_for_automation(
        &self,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        explicit_session_id: Option<&str>,
        objective: &str,
        next_expected_action: &str,
        policy: background_session::BackgroundSessionPolicy,
    ) -> Option<background_session::BackgroundSession> {
        if let Some(session_id) = explicit_session_id.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }) {
            if let Some(existing) = self.background_sessions.get(session_id).await {
                let rebound = self
                    .background_sessions
                    .bind_runtime_context(
                        &existing.id,
                        Some(channel),
                        conversation_id,
                        project_id,
                        Some("agent"),
                    )
                    .await
                    .unwrap_or(existing);
                let rebound = self
                    .seed_background_session_policy_if_unset(rebound, &policy)
                    .await;
                if let Some(cid) = conversation_id {
                    self.sync_background_session_artifact_context(cid, &rebound)
                        .await;
                }
                return Some(rebound);
            }
        }

        let conversation_id = conversation_id
            .map(str::trim)
            .filter(|value| !value.is_empty())?;

        let recent_artifacts = self.load_recent_artifact_contexts(conversation_id).await;
        let resolved = self
            .resolve_background_session_reference(
                conversation_id,
                objective,
                &recent_artifacts,
                false,
            )
            .await;
        if let Some(existing) = resolved.session {
            let rebound = self
                .background_sessions
                .bind_runtime_context(
                    &existing.id,
                    Some(channel),
                    Some(conversation_id),
                    project_id,
                    Some("agent"),
                )
                .await
                .unwrap_or(existing);
            let rebound = self
                .seed_background_session_policy_if_unset(rebound, &policy)
                .await;
            self.sync_background_session_artifact_context(conversation_id, &rebound)
                .await;
            return Some(rebound);
        }

        let session = self
            .background_sessions
            .create(
                background_session::BackgroundSessionCreate {
                    title: None,
                    objective: objective.to_string(),
                    summary: Some(
                        "Created automatically to keep background work attached to one durable session."
                            .to_string(),
                    ),
                    current_focus: Some(objective.to_string()),
                    waiting_on: None,
                    next_expected_action: Some(next_expected_action.to_string()),
                    working_memory: Some(format!(
                        "Objective: {}\nProvenance: automation request in conversation {}\n",
                        safe_truncate(objective, 400),
                        conversation_id
                    )),
                    preferred_delivery_channel: self.resolve_single_notification_channel(None).await,
                    channel: Some(channel.to_string()),
                    conversation_id: Some(conversation_id.to_string()),
                    project_id: project_id.map(|value| value.to_string()),
                    task_ids: Vec::new(),
                    watcher_ids: Vec::new(),
                    policy,
                },
                Some("agent"),
            )
            .await;
        self.sync_background_session_artifact_context(conversation_id, &session)
            .await;
        Some(session)
    }

    pub(super) async fn attach_items_to_background_session(
        &self,
        session_id: &str,
        task_ids: &[String],
        watcher_ids: &[String],
    ) {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return;
        }
        self.background_sessions
            .remove_child_references(task_ids, watcher_ids, Some("agent"))
            .await;
        let _ = self
            .background_sessions
            .attach_items(session_id, task_ids, watcher_ids, Some("agent"))
            .await;
    }

    pub(super) async fn sync_background_session_after_response(
        &self,
        conversation_id: &str,
        message: &str,
        response: &str,
    ) {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return;
        }
        let recent_artifacts = self.load_recent_artifact_contexts(conversation_id).await;
        let Some(session) = self
            .resolve_background_session_reference(
                conversation_id,
                message,
                &recent_artifacts,
                false,
            )
            .await
            .session
        else {
            return;
        };
        let next_expected_action = response
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| safe_truncate(line, 220));
        if let Some(updated) = self
            .background_sessions
            .apply_chat_turn(
                &session.id,
                message,
                response,
                next_expected_action,
                Some("agent"),
            )
            .await
        {
            self.sync_background_session_artifact_context(conversation_id, &updated)
                .await;
        }
    }

    pub(super) async fn maybe_consolidate_idle_background_sessions(&self) {
        let sessions = self.background_sessions.list().await;
        if sessions.is_empty() {
            return;
        }

        let now = chrono::Utc::now();
        let task_snapshot = { self.tasks.read().await.all().to_vec() };
        let watcher_snapshot = self.watcher_manager.list().await;
        let recent_runs = crate::core::list_automation_runs(&self.storage, 40)
            .await
            .unwrap_or_default();

        for session in sessions {
            if session.status.is_closed() {
                continue;
            }
            if (now - session.last_activity_at).num_minutes()
                < BACKGROUND_SESSION_IDLE_CONSOLIDATION_AFTER_MINS
            {
                continue;
            }
            if session
                .last_consolidated_at
                .map(|value| {
                    (now - value).num_minutes() < BACKGROUND_SESSION_CONSOLIDATION_COOLDOWN_MINS
                })
                .unwrap_or(false)
            {
                continue;
            }

            let mut lines = vec![
                format!("Session: {}", safe_truncate(&session.title, 120)),
                format!("Objective: {}", safe_truncate(&session.objective, 220)),
                format!("Status: {}", session.status.label()),
            ];
            if let Some(focus) = session
                .current_focus
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                lines.push(format!("Current focus: {}", safe_truncate(focus, 220)));
            }
            if let Some(waiting_on) = session
                .waiting_on
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                lines.push(format!("Waiting on: {}", safe_truncate(waiting_on, 220)));
            }

            let task_lines = task_snapshot
                .iter()
                .filter(|task| {
                    session
                        .linked_task_ids
                        .iter()
                        .any(|id| id == &task.id.to_string())
                        || background_session::background_session_id_from_automation(
                            &task.arguments,
                        )
                        .as_deref()
                            == Some(session.id.as_str())
                })
                .take(4)
                .map(|task| {
                    format!(
                        "- task {} [{}]: {}",
                        task.id,
                        Self::task_status_debug_label(&task.status),
                        safe_truncate(&task.description, 120)
                    )
                })
                .collect::<Vec<_>>();
            if !task_lines.is_empty() {
                lines.push("Linked tasks:".to_string());
                lines.extend(task_lines);
            }

            let watcher_lines = watcher_snapshot
                .iter()
                .filter(|watcher| {
                    session
                        .linked_watcher_ids
                        .iter()
                        .any(|id| id == &watcher.id.to_string())
                        || background_session::background_session_id_from_automation(
                            &watcher.poll_arguments,
                        )
                        .as_deref()
                            == Some(session.id.as_str())
                })
                .take(4)
                .map(|watcher| {
                    format!(
                        "- watcher {} [{}]: {}",
                        watcher.id,
                        Self::watcher_supervisor_status_label(&watcher.status),
                        safe_truncate(&watcher.description, 120)
                    )
                })
                .collect::<Vec<_>>();
            if !watcher_lines.is_empty() {
                lines.push("Linked watchers:".to_string());
                lines.extend(watcher_lines);
            }

            let run_lines = recent_runs
                .iter()
                .filter(|run| {
                    session
                        .linked_task_ids
                        .iter()
                        .chain(session.linked_watcher_ids.iter())
                        .any(|id| id == &run.automation_id)
                })
                .take(4)
                .map(|run| {
                    format!(
                        "- run {} [{}]: {}",
                        run.automation_id,
                        format!("{:?}", run.status).to_ascii_lowercase(),
                        safe_truncate(&run.title, 120)
                    )
                })
                .collect::<Vec<_>>();
            if !run_lines.is_empty() {
                lines.push("Recent runs:".to_string());
                lines.extend(run_lines);
            }

            let event_lines = session
                .events
                .iter()
                .rev()
                .take(4)
                .map(|event| {
                    format!(
                        "- {} [{}]: {}",
                        event.at.to_rfc3339(),
                        event.kind,
                        safe_truncate(&event.summary, 120)
                    )
                })
                .collect::<Vec<_>>();
            if !event_lines.is_empty() {
                lines.push("Recent session events:".to_string());
                lines.extend(event_lines);
            }

            lines.push(format!("Provenance: session_id={}", session.id));

            let summary = Some(
                session
                    .summary
                    .clone()
                    .unwrap_or_else(|| safe_truncate(&session.objective, 220)),
            );
            let next_expected_action = session.next_expected_action.clone().or_else(|| {
                if session.status == background_session::BackgroundSessionStatus::Paused {
                    Some(
                        "Resume the session when you want background work to continue.".to_string(),
                    )
                } else if session.status == background_session::BackgroundSessionStatus::Waiting {
                    Some("Wait for the external signal or provide the missing input.".to_string())
                } else {
                    Some("Review linked work and continue from the latest outcome.".to_string())
                }
            });

            let _ = self
                .background_sessions
                .record_consolidation(
                    &session.id,
                    summary,
                    lines.join("\n"),
                    next_expected_action,
                    Some("agent"),
                )
                .await;
        }
    }

    pub(crate) async fn enforce_background_session_policy_for_action(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> std::result::Result<(), String> {
        Self::enforce_background_session_policy_for_action_shared(
            &self.background_sessions,
            self.runtime.as_ref(),
            action_name,
            arguments,
        )
        .await
    }

    pub(crate) async fn enforce_background_session_policy_for_action_shared(
        background_sessions: &background_session::BackgroundSessionManager,
        runtime: &ActionRuntime,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> std::result::Result<(), String> {
        let Some(session_id) = background_session::background_session_id_from_automation(arguments)
        else {
            return Ok(());
        };
        let Some(session) = background_sessions.get(&session_id).await else {
            return Ok(());
        };
        let policy = session.policy.clone().normalized();
        if policy.is_unset() {
            return Ok(());
        }

        let actions = runtime.list_enabled_actions().await.unwrap_or_default();
        let metadata = actions
            .iter()
            .find(|action| action.name.eq_ignore_ascii_case(action_name.trim()))
            .map(crate::actions::ActionDef::planner_metadata)
            .unwrap_or_default();
        let role = planner_action_role_name(&metadata.role);
        let integration_class = planner_integration_class_name(&metadata.integration_class);
        if policy.allows(role, integration_class) {
            return Ok(());
        }

        Err(format!(
            "Background session policy blocks `{}` (role `{}`, integration `{}`) for session `{}`. Start a new background session or update the session policy from an authenticated interactive request.",
            safe_truncate(action_name.trim(), 80),
            role,
            integration_class,
            safe_truncate(&session.title, 120)
        ))
    }
}
