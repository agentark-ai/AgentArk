use super::*;

impl Agent {
    pub(super) fn pending_resilience_followup_key(conversation_id: &str) -> String {
        format!("pending_resilience_followup:{}", conversation_id.trim())
    }

    pub(super) fn pending_resilience_followup_is_expired(requested_at: &str) -> bool {
        chrono::DateTime::parse_from_rfc3339(requested_at)
            .map(|value| {
                chrono::Utc::now().signed_duration_since(value.with_timezone(&chrono::Utc))
                    > chrono::Duration::hours(PENDING_RESILIENCE_FOLLOWUP_TTL_HOURS)
            })
            .unwrap_or(false)
    }

    pub(super) async fn remember_pending_resilience_followup(
        &self,
        conversation_id: &str,
        message: &str,
        channel: &str,
        project_id: Option<&str>,
        outcome: &crate::core::UserFacingOutcome,
    ) {
        if conversation_id.trim().is_empty() {
            return;
        }

        let payload = PendingResilienceFollowup {
            request_state: outcome.request_state.clone(),
            original_message: safe_truncate(message, 4000),
            assistant_message: safe_truncate(&outcome.message, 4000),
            channel: channel.to_string(),
            project_id: project_id.map(|value| value.to_string()),
            reason_code: outcome.reason_code.clone(),
            requested_at: chrono::Utc::now().to_rfc3339(),
        };
        let Ok(encoded) = serde_json::to_vec(&payload) else {
            return;
        };
        let key = Self::pending_resilience_followup_key(conversation_id);
        let _ = self.storage.set_encrypted(&key, &encoded).await;
    }

    pub(super) async fn load_pending_resilience_followup(
        &self,
        conversation_id: &str,
    ) -> Option<PendingResilienceFollowup> {
        if conversation_id.trim().is_empty() {
            return None;
        }
        let key = Self::pending_resilience_followup_key(conversation_id);
        let raw = self.storage.get_encrypted(&key).await.ok().flatten()?;
        let pending = serde_json::from_slice::<PendingResilienceFollowup>(&raw).ok()?;
        if Self::pending_resilience_followup_is_expired(&pending.requested_at) {
            let _ = self.storage.delete(&key).await;
            return None;
        }
        Some(pending)
    }

    pub(super) async fn clear_pending_resilience_followup(&self, conversation_id: &str) {
        if conversation_id.trim().is_empty() {
            return;
        }
        let key = Self::pending_resilience_followup_key(conversation_id);
        let _ = self.storage.delete(&key).await;
    }

    pub(super) fn outcome_requires_pending_resilience_followup(
        outcome: &crate::core::UserFacingOutcome,
    ) -> bool {
        matches!(
            outcome.status,
            crate::core::UserFacingOutcomeStatus::NeedsClarification
                | crate::core::UserFacingOutcomeStatus::NeedsPermission
                | crate::core::UserFacingOutcomeStatus::NeedsIntegration
                | crate::core::UserFacingOutcomeStatus::NeedsCredentials
                | crate::core::UserFacingOutcomeStatus::NeedsStrongerModel
        )
    }

    pub(super) async fn sync_pending_resilience_followup(
        &self,
        conversation_id: &str,
        message: &str,
        channel: &str,
        project_id: Option<&str>,
        outcome: &crate::core::UserFacingOutcome,
    ) {
        if Self::outcome_requires_pending_resilience_followup(outcome) {
            self.remember_pending_resilience_followup(
                conversation_id,
                message,
                channel,
                project_id,
                outcome,
            )
            .await;
        } else {
            self.clear_pending_resilience_followup(conversation_id)
                .await;
        }
    }

    pub(super) fn pending_resilience_followup_summary(
        pending: &PendingResilienceFollowup,
    ) -> Option<String> {
        let blocker = safe_truncate(&pending.assistant_message, 220);
        match pending.request_state {
            super::RequestState::NeedsClarification => Some(format!(
                "Resume the previously blocked request if the user supplies the missing clarification or detail. Last blocker: {}",
                blocker
            )),
            super::RequestState::NeedsPermission => Some(format!(
                "Resume the previously blocked request if the user approves proceeding now. Last blocker: {}",
                blocker
            )),
            super::RequestState::NeedsIntegration => Some(format!(
                "Retry the previously blocked request after the requested integration or setup step. Last blocker: {}",
                blocker
            )),
            super::RequestState::NeedsCredentials => Some(format!(
                "Retry the previously blocked request after the required credentials are configured. Last blocker: {}",
                blocker
            )),
            super::RequestState::NeedsStrongerModel => Some(format!(
                "Retry the previously blocked request after the model configuration is upgraded. Last blocker: {}",
                blocker
            )),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub(super) fn tool_output_requires_sensitive_context_approval(
        &self,
        output: &str,
        allow_sensitive_context: bool,
    ) -> Option<Vec<String>> {
        let result = crate::security::sanitize_model_input_text(
            output,
            &self.config.model_privacy,
            crate::security::ModelInputContext::ToolOutput,
            allow_sensitive_context,
        );
        if result.decision == crate::security::ModelInputPrivacyDecision::RequiresApproval {
            Some(if result.reasons.is_empty() {
                vec!["Sensitive person-linked data was detected in tool output.".to_string()]
            } else {
                result.reasons
            })
        } else {
            None
        }
    }
}
