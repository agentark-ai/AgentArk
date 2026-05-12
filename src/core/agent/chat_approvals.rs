use super::*;
use anyhow::Context as _;

const DIRECT_CHAT_APPROVAL_TTL_MINS: i64 = 30;
const DIRECT_CHAT_CHAIN_APPROVAL_ACTION: &str = "__agentark_direct_chat_chain__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectChatApprovalSubmitDecision {
    Approve,
    Reject,
}

impl DirectChatApprovalSubmitDecision {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Reject => "reject",
        }
    }
}

pub(crate) fn parse_direct_chat_approval_submit_text(
    input: &str,
) -> Option<(String, DirectChatApprovalSubmitDecision)> {
    let parts = input
        .trim()
        .split(':')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 3 {
        return None;
    }

    let id_part = *parts.last()?;
    let approval_id = uuid::Uuid::parse_str(id_part).ok()?.to_string();
    let protocol = parts[..parts.len().saturating_sub(1)]
        .join(":")
        .to_ascii_lowercase();
    if !protocol.contains("direct_chat") || !protocol.contains("approval") {
        return None;
    }

    let decision = parts[..parts.len().saturating_sub(1)]
        .iter()
        .rev()
        .find_map(|part| match part.to_ascii_lowercase().as_str() {
            "approve" => Some(DirectChatApprovalSubmitDecision::Approve),
            "reject" => Some(DirectChatApprovalSubmitDecision::Reject),
            _ => None,
        })?;
    Some((approval_id, decision))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PersistedDirectChatChainApproval {
    conversation_id: Option<String>,
    request_channel: String,
    authorization: crate::actions::ActionAuthorizationContext,
    reason: String,
    requested_at: String,
    expires_at: String,
    calls: Vec<DirectChatChainApprovalCall>,
}

fn redact_direct_chat_approval_preview_value(
    key: Option<&str>,
    value: &serde_json::Value,
    depth: usize,
) -> serde_json::Value {
    if key.is_some_and(is_sensitive_tool_call_argument_key) {
        return serde_json::json!("[redacted]");
    }
    if depth >= 3 {
        return match value {
            serde_json::Value::Array(items) => {
                serde_json::json!(format!(
                    "[{} item{}]",
                    items.len(),
                    if items.len() == 1 { "" } else { "s" }
                ))
            }
            serde_json::Value::Object(map) => {
                serde_json::json!(format!(
                    "{{{} field{}}}",
                    map.len(),
                    if map.len() == 1 { "" } else { "s" }
                ))
            }
            serde_json::Value::String(text) => serde_json::json!(safe_truncate(text, 160)),
            other => other.clone(),
        };
    }

    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys.into_iter().take(16) {
                if let Some(value) = map.get(&key) {
                    out.insert(
                        key.clone(),
                        redact_direct_chat_approval_preview_value(Some(&key), value, depth + 1),
                    );
                }
            }
            if map.len() > 16 {
                out.insert(
                    "_omitted".to_string(),
                    serde_json::json!(map.len().saturating_sub(16)),
                );
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .take(8)
                .map(|item| redact_direct_chat_approval_preview_value(None, item, depth + 1))
                .collect(),
        ),
        serde_json::Value::String(text) => serde_json::json!(safe_truncate(text, 240)),
        other => other.clone(),
    }
}

fn direct_chat_chain_approval_is_expired(request: &PersistedDirectChatChainApproval) -> bool {
    chrono::DateTime::parse_from_rfc3339(&request.expires_at)
        .map(|expires_at| expires_at.with_timezone(&chrono::Utc) <= chrono::Utc::now())
        .unwrap_or(true)
}

fn direct_chat_chain_approval_view(
    id: &str,
    request: &PersistedDirectChatChainApproval,
) -> DirectChatApprovalView {
    let steps = request
        .calls
        .iter()
        .map(|call| DirectChatApprovalStepView {
            action_name: call.action_name.clone(),
            arguments_preview: redact_direct_chat_approval_preview_value(None, &call.arguments, 0),
        })
        .collect::<Vec<_>>();
    let action_name = if request.calls.len() == 1 {
        request.calls[0].action_name.clone()
    } else {
        "action_chain".to_string()
    };
    let arguments_preview = if request.calls.len() == 1 {
        redact_direct_chat_approval_preview_value(None, &request.calls[0].arguments, 0)
    } else {
        serde_json::json!({
            "step_count": request.calls.len(),
            "actions": request
                .calls
                .iter()
                .map(|call| call.action_name.as_str())
                .collect::<Vec<_>>(),
        })
    };
    DirectChatApprovalView {
        id: id.to_string(),
        action_name,
        reason: request.reason.clone(),
        requested_at: request.requested_at.clone(),
        expires_at: request.expires_at.clone(),
        arguments_preview,
        steps,
    }
}

fn direct_chat_approval_choice(
    request: &DirectChatApprovalView,
    decision: &str,
    label: &str,
) -> ClarificationChoice {
    let kind = if request.steps.is_empty() {
        "direct_chat_approval"
    } else {
        "direct_chat_chain_approval"
    };
    ClarificationChoice {
        label: label.to_string(),
        submit_text: format!("{kind}:{decision}:{}", request.id),
        kind: Some(kind.to_string()),
        approval: Some(DirectChatApprovalChoice {
            id: request.id.clone(),
            decision: decision.to_string(),
            action_name: request.action_name.clone(),
            steps: request.steps.clone(),
        }),
    }
}

fn compact_direct_chat_action_result(result: &str) -> String {
    let trimmed = result.trim();
    if trimmed.is_empty() {
        return "The action completed with no output.".to_string();
    }
    if let Some(summary) = super::tool_responses::summarize_structured_tool_output_for_user(trimmed)
    {
        return summary;
    }
    safe_truncate(trimmed, 12_000)
}

fn direct_chat_structured_completion_value(result: &str) -> Option<serde_json::Value> {
    let trimmed = result.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(payload) = trimmed
        .trim_start()
        .strip_prefix(crate::runtime::TOOL_COMPLETION_MARKER)
    {
        let payload = payload.lines().next().unwrap_or(payload).trim();
        return serde_json::from_str::<serde_json::Value>(payload).ok();
    }
    let value = serde_json::from_str::<serde_json::Value>(trimmed).ok()?;
    if let Some(result) = value.get("result").and_then(|inner| inner.as_str()) {
        if let Some(inner) = direct_chat_structured_completion_value(result) {
            return Some(inner);
        }
    }
    Some(value)
}

fn completion_text_field<'a>(
    value: &'a serde_json::Value,
    key: &str,
) -> Option<&'a str> {
    value
        .get(key)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|item| !item.is_empty())
}

fn completion_data(value: &serde_json::Value) -> Option<&serde_json::Value> {
    value.get("data")
}

fn direct_chat_completion_reference_lines(data: &serde_json::Value) -> Vec<String> {
    let Some(object) = data.as_object() else {
        return Vec::new();
    };
    let preferred = [
        ("watcher_id", "Watcher ID"),
        ("task_id", "Task ID"),
        ("background_session_id", "Background session ID"),
        ("session_id", "Session ID"),
    ];
    preferred
        .iter()
        .filter_map(|(key, label)| {
            object
                .get(*key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("- {}: {}", label, value))
        })
        .collect()
}

fn format_watch_approval_result(value: &serde_json::Value) -> Option<String> {
    let status = completion_text_field(value, "status")
        .unwrap_or("completed")
        .to_ascii_lowercase();
    if !matches!(
        status.as_str(),
        "completed" | "complete" | "succeeded" | "success" | "ok" | "executed"
    ) {
        return None;
    }
    let detail = completion_text_field(value, "detail").unwrap_or("");
    let data = completion_data(value);
    let updated = detail.to_ascii_lowercase().contains("updated");
    let mut lines = Vec::new();
    lines.push(if updated {
        "I updated the background watcher.".to_string()
    } else {
        "I created the background watcher.".to_string()
    });

    let mut facts = Vec::new();
    if let Some(description) = data.and_then(|data| completion_text_field(data, "description")) {
        facts.push(format!("- Purpose: {}", safe_truncate(description, 220)));
    }
    if let Some(cadence) = data.and_then(|data| completion_text_field(data, "cadence")) {
        facts.push(format!("- Cadence: {}", cadence));
    }
    if let Some(notification) = data.and_then(|data| completion_text_field(data, "notification")) {
        facts.push(format!("- Notifications: {}", notification));
    }
    if let Some(duration) = data.and_then(|data| completion_text_field(data, "duration")) {
        facts.push(format!("- Duration: {}", duration));
    }
    if !facts.is_empty() {
        lines.push(String::new());
        lines.extend(facts);
    }
    lines.push(String::new());
    lines.push("You can view, pause, edit, or stop it from Background Work.".to_string());

    let refs = data
        .map(direct_chat_completion_reference_lines)
        .unwrap_or_default();
    if !refs.is_empty() {
        lines.push(String::new());
        lines.extend(refs);
    }
    Some(lines.join("\n"))
}

fn format_schedule_approval_result(value: &serde_json::Value) -> Option<String> {
    let status = completion_text_field(value, "status")
        .unwrap_or("completed")
        .to_ascii_lowercase();
    if !matches!(
        status.as_str(),
        "completed" | "complete" | "succeeded" | "success" | "ok" | "executed"
    ) {
        return None;
    }
    let data = completion_data(value);
    let detail = completion_text_field(value, "detail")
        .map(|value| safe_truncate(value, 260))
        .unwrap_or_else(|| "The scheduled work was created.".to_string());
    let mut lines = vec![
        "I scheduled the requested work.".to_string(),
        String::new(),
        format!("- Summary: {}", detail),
        "You can view or manage it from Tasks and Background Work.".to_string(),
    ];
    let refs = data
        .map(direct_chat_completion_reference_lines)
        .unwrap_or_default();
    if !refs.is_empty() {
        lines.push(String::new());
        lines.extend(refs);
    }
    Some(lines.join("\n"))
}

fn format_direct_chat_approval_action_result(action_name: &str, result: &str) -> String {
    if let Some(value) = direct_chat_structured_completion_value(result) {
        let tool = completion_text_field(&value, "tool").unwrap_or(action_name);
        if tool.eq_ignore_ascii_case("watch") {
            if let Some(formatted) = format_watch_approval_result(&value) {
                return formatted;
            }
        }
        if tool.eq_ignore_ascii_case("schedule_task") {
            if let Some(formatted) = format_schedule_approval_result(&value) {
                return formatted;
            }
        }
    }

    let summary = compact_direct_chat_action_result(result);
    format!("`{}` completed.\n{}", action_name, summary)
}

impl Agent {
    pub(crate) async fn remember_direct_chat_chain_approval(
        &self,
        conversation_id: Option<&str>,
        request_channel: &str,
        calls: &[DirectChatChainApprovalCall],
        authorization: &crate::actions::ActionAuthorizationContext,
        reason: &str,
    ) -> Result<DirectChatApprovalView> {
        if calls.is_empty() {
            anyhow::bail!("Approval request must contain at least one action");
        }
        let now = chrono::Utc::now();
        let expires_at = now + chrono::Duration::minutes(DIRECT_CHAT_APPROVAL_TTL_MINS);
        let id = uuid::Uuid::new_v4().to_string();
        let request = PersistedDirectChatChainApproval {
            conversation_id: conversation_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            request_channel: request_channel.trim().to_string(),
            authorization: authorization.clone(),
            reason: reason.trim().to_string(),
            requested_at: now.to_rfc3339(),
            expires_at: expires_at.to_rfc3339(),
            calls: calls.to_vec(),
        };
        let serialized = serde_json::to_string(&request)
            .context("failed to serialize direct chat chain approval")?;
        self.storage
            .upsert_approval_request(
                &id,
                DIRECT_CHAT_CHAIN_APPROVAL_ACTION,
                &serialized,
                "direct_chat_chain_explicit_user_approval",
                &request.requested_at,
            )
            .await?;
        Ok(direct_chat_chain_approval_view(&id, &request))
    }

    pub(crate) fn direct_chat_approval_choices(
        &self,
        request: &DirectChatApprovalView,
    ) -> Vec<ClarificationChoice> {
        vec![
            direct_chat_approval_choice(request, "approve", "Approve"),
            direct_chat_approval_choice(request, "reject", "Reject"),
        ]
    }

    async fn load_direct_chat_chain_approval(
        &self,
        approval_id: &str,
    ) -> Result<
        Option<(
            crate::storage::entities::approval_log::Model,
            PersistedDirectChatChainApproval,
        )>,
    > {
        let Some(row) = self.storage.get_approval_request(approval_id).await? else {
            return Ok(None);
        };
        if row.action_name != DIRECT_CHAT_CHAIN_APPROVAL_ACTION {
            return Ok(None);
        }
        let request = serde_json::from_str::<PersistedDirectChatChainApproval>(&row.arguments)
            .with_context(|| format!("failed to decode approval request `{approval_id}`"))?;
        Ok(Some((row, request)))
    }

    pub(crate) async fn reject_direct_chat_any_approval(
        &self,
        approval_id: &str,
    ) -> Result<(DirectChatApprovalView, String)> {
        let Some((row, request)) = self.load_direct_chat_chain_approval(approval_id).await? else {
            anyhow::bail!("Approval request not found or already handled");
        };
        if row.status != "pending" {
            anyhow::bail!("Approval request not found or already handled");
        }
        let view = direct_chat_chain_approval_view(approval_id, &request);
        self.storage
            .resolve_approval_request(approval_id, "denied", "user")
            .await?;
        let response =
            self.filter_direct_chat_approval_response(&format!("Rejected `{}`.", view.action_name));
        self.persist_direct_chat_approval_assistant_message(
            request.conversation_id.as_deref(),
            &response,
        )
        .await?;
        Ok((view, response))
    }

    pub(super) async fn retire_pending_direct_chat_approvals_for_new_intent(
        &self,
        conversation_id: &str,
        current_message: &str,
    ) {
        if conversation_id.trim().is_empty()
            || parse_direct_chat_approval_submit_text(current_message).is_some()
        {
            return;
        }
        let rows = match self.storage.get_approval_log(64, 0).await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "failed to load approval log for pending approval cleanup"
                );
                return;
            }
        };
        for row in rows {
            if row.status != "pending" || row.action_name != DIRECT_CHAT_CHAIN_APPROVAL_ACTION {
                continue;
            }
            let Ok(request) =
                serde_json::from_str::<PersistedDirectChatChainApproval>(&row.arguments)
            else {
                continue;
            };
            if request
                .conversation_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                != Some(conversation_id.trim())
            {
                continue;
            }
            let status = if direct_chat_chain_approval_is_expired(&request) {
                "expired"
            } else {
                "superseded"
            };
            if let Err(error) = self
                .storage
                .resolve_approval_request(&row.id, status, "new_turn")
                .await
            {
                tracing::debug!(
                    approval_id = %row.id,
                    error = %error,
                    "failed to retire stale direct chat approval"
                );
            }
        }
    }

    fn filter_direct_chat_approval_response(&self, response: &str) -> String {
        self.security.filter_output(response).text
    }

    async fn persist_direct_chat_approval_assistant_message(
        &self,
        conversation_id: Option<&str>,
        response: &str,
    ) -> Result<()> {
        let Some(conversation_id) = conversation_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(());
        };
        let response = self.filter_direct_chat_approval_response(response);
        let msg = crate::storage::entities::message::Model {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation_id.to_string(),
            role: "assistant".to_string(),
            content: response.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            model_used: Some("approval".to_string()),
            trace_id: None,
        };
        self.encrypted_storage
            .insert_message_encrypted_if_absent(&msg)
            .await?;
        {
            let mut history = self.conversation_history.write().await;
            let conversation_history = history
                .entry(conversation_id.to_string())
                .or_insert_with(Vec::new);
            conversation_history.push(ConversationMessage {
                role: "assistant".to_string(),
                content: response,
                _timestamp: chrono::Utc::now(),
            });
            self.trim_in_memory_conversation_history(conversation_history);
        }
        Ok(())
    }

    /// Build a structured JSON envelope the executor returns as a tool result
    /// when a cross-layer policy rule requires user approval. The envelope is
    /// shape-typed (no surface phrasing); the LLM uses semantic understanding
    /// to write the user-facing message and the frontend renders the inline
    /// approval choices from the `inline_choices` array.
    ///
    /// This persists a pending approval row keyed by a generated approval_id
    /// so the user's button click can re-run the approved chain.
    pub(crate) async fn build_approval_required_envelope(
        &self,
        conversation_id: Option<&str>,
        request_channel: &str,
        calls: &[DirectChatChainApprovalCall],
        authorization: &crate::actions::ActionAuthorizationContext,
        rule_id: &str,
        reason: &str,
    ) -> Result<serde_json::Value> {
        let view = self
            .remember_direct_chat_chain_approval(
                conversation_id,
                request_channel,
                calls,
                authorization,
                reason,
            )
            .await?;
        let choices = self.direct_chat_approval_choices(&view);
        let inline_choices = choices
            .iter()
            .map(|choice| {
                serde_json::json!({
                    "label": choice.label,
                    "submit_text": choice.submit_text,
                    "kind": choice.kind,
                    "approval": choice.approval,
                })
            })
            .collect::<Vec<_>>();
        Ok(serde_json::json!({
            "status": "approval_required",
            "approval_id": view.id,
            "rule_id": rule_id,
            "reason": view.reason,
            "expires_at": view.expires_at,
            "action_name": view.action_name,
            "arguments_preview": view.arguments_preview,
            "steps": view.steps,
            "inline_choices": inline_choices,
            "remediation": {"type": "approve", "target": view.id},
        }))
    }

    pub(crate) async fn approve_direct_chat_any_approval(
        &self,
        approval_id: &str,
    ) -> Result<(DirectChatApprovalView, String)> {
        let Some((row, request)) = self.load_direct_chat_chain_approval(approval_id).await? else {
            anyhow::bail!("Approval request not found or already handled");
        };
        if row.status != "pending" {
            anyhow::bail!("Approval request not found or already handled");
        }
        if direct_chat_chain_approval_is_expired(&request) {
            self.storage
                .resolve_approval_request(approval_id, "expired", "auto_timeout")
                .await?;
            anyhow::bail!("Approval request expired. Ask the agent to run the actions again.");
        }
        let view = direct_chat_chain_approval_view(approval_id, &request);
        let mut authorization = request.authorization.clone();
        authorization.current_turn_is_explicit_approval = true;
        let mut outputs = Vec::new();
        for call in request.calls.iter() {
            let result = match self
                .execute_action_with_hooks(
                    &call.action_name,
                    &call.arguments,
                    &request.request_channel,
                    None,
                    Some(&authorization),
                    request.conversation_id.as_deref(),
                    None,
                )
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    self.storage
                        .resolve_approval_request(approval_id, "failed", "system")
                        .await?;
                    anyhow::bail!(
                        "Approved action chain failed while running `{}`: {}",
                        call.action_name,
                        error
                    );
                }
            };
            outputs.push(format_direct_chat_approval_action_result(
                &call.action_name,
                &result,
            ));
        }
        self.storage
            .resolve_approval_request(approval_id, "approved", "user")
            .await?;
        let response = if outputs.len() == 1 {
            format!("Approved. {}", outputs[0].trim())
        } else {
            let numbered = outputs
                .iter()
                .enumerate()
                .map(|(index, output)| format!("{}. {}", index + 1, output.trim()))
                .collect::<Vec<_>>()
                .join("\n\n");
            format!("Approved. I completed {} actions.\n\n{}", outputs.len(), numbered)
        };
        let response = self.filter_direct_chat_approval_response(&response);
        self.persist_direct_chat_approval_assistant_message(
            request.conversation_id.as_deref(),
            &response,
        )
        .await?;
        Ok((view, response))
    }
}
