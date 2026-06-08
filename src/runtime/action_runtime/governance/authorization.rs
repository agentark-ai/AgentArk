use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) fn normalize_action_definition(info: ActionDef) -> ActionDef {
        let mut normalized = info;
        normalized.authorization = Self::merged_authorization_for_action(&normalized);
        normalized
    }

    pub(in crate::runtime) fn merged_authorization_for_action(
        info: &ActionDef,
    ) -> ActionAuthorization {
        let defaults = Self::default_authorization_for_action(info);
        let mut authorization = info.authorization.clone();
        if matches!(authorization.risk_level, ActionRiskLevel::None) {
            authorization.risk_level = defaults.risk_level;
        }
        if !authorization.requires_auth {
            authorization.requires_auth = defaults.requires_auth;
        }
        if authorization.allowed_roles.is_empty() {
            authorization.allowed_roles = defaults.allowed_roles;
        }
        if authorization.rate_limit.is_none() {
            authorization.rate_limit = defaults.rate_limit;
        }
        if !authorization.human_approval.required {
            authorization.human_approval.required = defaults.human_approval.required;
        }
        authorization
    }

    pub(in crate::runtime) fn default_authorization_for_action(
        info: &ActionDef,
    ) -> ActionAuthorization {
        let lowered = info.name.trim().to_ascii_lowercase();
        let dangerous = Self::action_has_dangerous_capabilities(&info.capabilities);
        let background_sensitive =
            BACKGROUND_BLOCKED_ACTIONS.contains(&lowered.as_str()) || dangerous;

        if background_sensitive {
            return ActionAuthorization {
                risk_level: ActionRiskLevel::High,
                requires_auth: true,
                ..Default::default()
            };
        }

        ActionAuthorization::default()
    }

    pub(in crate::runtime) fn action_has_dangerous_capabilities(capabilities: &[String]) -> bool {
        capabilities.iter().any(|cap| {
            let permission = crate::security::action_guard::ActionGuard::parse_permission(cap);
            !matches!(
                permission,
                crate::security::action_guard::Permission::Custom(_)
            ) && matches!(
                crate::security::action_guard::ActionGuard::permission_risk(&permission),
                crate::security::action_guard::PermissionRisk::Dangerous
            )
        })
    }

    pub(in crate::runtime) fn is_background_surface(surface: &ActionExecutionSurface) -> bool {
        matches!(
            surface,
            ActionExecutionSurface::Automation | ActionExecutionSurface::Background
        )
    }

    pub(in crate::runtime) fn direct_trusted_chat_tool_override(
        auth_context: &ActionAuthorizationContext,
    ) -> bool {
        matches!(auth_context.surface, ActionExecutionSurface::Chat)
            && auth_context.direct_user_intent
            && auth_context
                .principal
                .as_ref()
                .is_some_and(|principal| principal.trusted)
    }

    pub(in crate::runtime) fn risk_rank(level: &ActionRiskLevel) -> u8 {
        match level {
            ActionRiskLevel::None => 0,
            ActionRiskLevel::Low => 1,
            ActionRiskLevel::Medium => 2,
            ActionRiskLevel::High => 3,
            ActionRiskLevel::Critical => 4,
        }
    }

    pub(in crate::runtime) fn truncate_audit_text(raw: &str, max_chars: usize) -> String {
        let redacted = crate::security::redact_pii(raw);
        let mut truncated = redacted.chars().take(max_chars).collect::<String>();
        if redacted.chars().count() > max_chars {
            truncated.push_str("...");
        }
        truncated
    }

    pub(in crate::runtime) fn normalize_optional_audit_text(
        raw: Option<&str>,
        max_chars: usize,
    ) -> Option<String> {
        raw.map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Self::truncate_audit_text(value, max_chars))
    }

    pub(in crate::runtime) async fn log_authorization_audit(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        authorization: &ActionAuthorization,
        auth_context: &ActionAuthorizationContext,
        decision: &ActionAuthorizationDecision,
    ) {
        let Some(storage) = self.storage() else {
            return;
        };
        let principal_payload = auth_context.principal.as_ref().map(|principal| {
            serde_json::json!({
                "user_id": principal.user_id,
                "role": principal.role,
                "auth_source": principal.auth_source,
                "trusted": principal.trusted,
            })
        });
        let payload = serde_json::json!({
            "surface": auth_context.surface.as_key(),
            "direct_user_intent": auth_context.direct_user_intent,
            "current_turn_is_explicit_approval": auth_context.current_turn_is_explicit_approval,
            "principal": principal_payload,
            "authorization": authorization,
            "decision": {
                "allowed": decision.allowed,
                "reason": decision.reason,
                "matched_role": decision.matched_role,
                "rate_limit_key": decision.rate_limit_key,
            }
        });
        let arguments_text = serde_json::to_string(arguments)
            .ok()
            .map(|value| Self::truncate_audit_text(&value, 1200));
        let payload_text = serde_json::to_string(&payload)
            .ok()
            .map(|value| Self::truncate_audit_text(&value, 2000));
        let row = crate::storage::entities::operational_log::Model {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            trace_id: None,
            conversation_id: None,
            channel: Self::truncate_audit_text(auth_context.surface.as_key(), 64),
            event_type: "tool_authorization".to_string(),
            success: decision.allowed,
            outcome: Self::truncate_audit_text(
                if decision.allowed {
                    "allowed"
                } else {
                    "blocked"
                },
                64,
            ),
            tool_name: Some(Self::truncate_audit_text(action_name, 128)),
            latency_ms: None,
            arguments: arguments_text,
            payload: payload_text,
            strategy_version: None,
            policy_version: None,
            prompt_version: None,
            model_slot: Self::normalize_optional_audit_text(
                auth_context
                    .principal
                    .as_ref()
                    .map(|principal| principal.auth_source.as_str()),
                128,
            ),
        };
        if let Err(error) = storage.insert_operational_log(&row).await {
            tracing::debug!("Failed to insert authorization audit log: {}", error);
        }
    }

    pub(in crate::runtime) fn capability_context_key(
        auth_context: &ActionAuthorizationContext,
    ) -> Option<String> {
        auth_context
            .capability_context_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(256).collect::<String>())
    }

    pub(in crate::runtime) fn prune_capability_run_contexts(
        contexts: &mut HashMap<String, CapabilityRunCorrelationRecord>,
    ) {
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(CAPABILITY_CONTEXT_TTL_SECS);
        contexts.retain(|_, record| record.updated_at >= cutoff);
        while contexts.len() > CAPABILITY_CONTEXT_LIMIT {
            let Some(oldest_key) = contexts
                .iter()
                .min_by_key(|(_, record)| record.updated_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            contexts.remove(&oldest_key);
        }
    }

    pub(in crate::runtime) fn capability_correlation_message(
        action_name: &str,
        decision: &crate::security::capabilities::CapabilityCorrelationDecision,
    ) -> String {
        let matched_rule_message = decision
            .report
            .as_ref()
            .and_then(|report| report.matched_rules.first())
            .map(|rule| rule.message.as_str());
        let detail = matched_rule_message
            .or(decision.message.as_deref())
            .unwrap_or(match decision.effect {
                crate::security::capabilities::CapabilityCorrelationEffect::Block => {
                    "This combination is not allowed by the active safety policy."
                }
                crate::security::capabilities::CapabilityCorrelationEffect::RequireApproval => {
                    "This combination needs approval before it can run."
                }
                crate::security::capabilities::CapabilityCorrelationEffect::Allow => {
                    "Allowed by capability policy."
                }
            });
        match decision.effect {
            crate::security::capabilities::CapabilityCorrelationEffect::Block => {
                format!(
                    "This action is blocked by security policy before running `{}`. {}",
                    action_name, detail
                )
            }
            crate::security::capabilities::CapabilityCorrelationEffect::RequireApproval => {
                format!(
                    "This action needs your approval before running `{}`. {}",
                    action_name, detail
                )
            }
            crate::security::capabilities::CapabilityCorrelationEffect::Allow => detail.to_string(),
        }
    }

    pub(in crate::runtime) fn nested_orchestration_action_request(
        parent_arguments: &serde_json::Value,
        item_arguments: &serde_json::Value,
        action_key: &str,
        arguments_key: &str,
    ) -> Option<(String, serde_json::Value)> {
        let action_name = item_arguments
            .get(action_key)
            .or_else(|| parent_arguments.get(action_key))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_string();
        let action_arguments = item_arguments
            .get(arguments_key)
            .or_else(|| parent_arguments.get(arguments_key))
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        Some((action_name, action_arguments))
    }

    pub(in crate::runtime) fn nested_orchestration_action_requests(
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Vec<(String, serde_json::Value)> {
        let mut requests = Vec::new();
        let mut seen = HashSet::new();
        let mut collect = |item_arguments: &serde_json::Value| {
            for (action_key, arguments_key) in [
                ("poll_action", "poll_arguments"),
                ("action", "action_arguments"),
            ] {
                let Some((nested_action, nested_arguments)) =
                    Self::nested_orchestration_action_request(
                        arguments,
                        item_arguments,
                        action_key,
                        arguments_key,
                    )
                else {
                    continue;
                };
                if nested_action.eq_ignore_ascii_case(action_name) {
                    continue;
                }
                let signature = format!(
                    "{}:{}",
                    nested_action.to_ascii_lowercase(),
                    serde_json::to_string(&nested_arguments).unwrap_or_default()
                );
                if seen.insert(signature) {
                    requests.push((nested_action, nested_arguments));
                }
            }
        };

        if let Some(items) = arguments.get("items").and_then(|value| value.as_array()) {
            for item in items {
                collect(item);
            }
        } else {
            collect(arguments);
        }
        requests
    }

    pub(in crate::runtime) async fn authorization_observations_for_invocation(
        &self,
        action_name: &str,
        action_def: &ActionDef,
        arguments: &serde_json::Value,
    ) -> Vec<crate::security::capabilities::CapabilityObservation> {
        let mut observations = crate::security::capabilities::observations_from_action_def(
            "runtime",
            action_def,
            Some(arguments),
        );
        for (nested_action_name, nested_arguments) in
            Self::nested_orchestration_action_requests(action_name, arguments)
        {
            let Some(nested_action_def) = self.action_definition(&nested_action_name).await else {
                continue;
            };
            observations.extend(crate::security::capabilities::observations_from_action_def(
                "runtime",
                &nested_action_def,
                Some(&nested_arguments),
            ));
        }
        observations
    }

    pub(in crate::runtime) async fn authorize_capability_correlation(
        &self,
        action_name: &str,
        action_def: &ActionDef,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Option<ActionAuthorizationDecision> {
        if matches!(
            auth_context.surface,
            ActionExecutionSurface::Internal | ActionExecutionSurface::Test
        ) {
            return None;
        }
        let direct_trusted_chat = Self::direct_trusted_chat_tool_override(auth_context);
        let context_key = Self::capability_context_key(auth_context)?;
        let candidate = self
            .authorization_observations_for_invocation(action_name, action_def, arguments)
            .await;
        if candidate.is_empty() {
            return None;
        }

        let mut contexts = self.capability_run_contexts.write().await;
        Self::prune_capability_run_contexts(&mut contexts);
        let record =
            contexts
                .entry(context_key.clone())
                .or_insert_with(|| CapabilityRunCorrelationRecord {
                    updated_at: chrono::Utc::now(),
                    context: crate::security::capabilities::RunCapabilityContext::default(),
                });
        let decision = crate::security::capabilities::evaluate_capability_correlation(
            record.context.observations(),
            &candidate,
        );
        match decision.effect {
            crate::security::capabilities::CapabilityCorrelationEffect::Allow => {
                record.context.extend(candidate);
                record
                    .context
                    .retain_recent(CAPABILITY_CONTEXT_OBSERVATION_LIMIT);
                record.updated_at = chrono::Utc::now();
                None
            }
            crate::security::capabilities::CapabilityCorrelationEffect::Block => {
                if direct_trusted_chat {
                    record.context.extend(candidate);
                    record
                        .context
                        .retain_recent(CAPABILITY_CONTEXT_OBSERVATION_LIMIT);
                    record.updated_at = chrono::Utc::now();
                    drop(contexts);
                    self.record_capability_correlation_decision(
                        action_name,
                        &context_key,
                        "direct_chat_allowed",
                        &decision,
                    )
                    .await;
                    return None;
                }
                drop(contexts);
                self.record_capability_correlation_decision(
                    action_name,
                    &context_key,
                    "blocked",
                    &decision,
                )
                .await;
                Some(ActionAuthorizationDecision::deny(
                    Self::capability_correlation_message(action_name, &decision),
                ))
            }
            crate::security::capabilities::CapabilityCorrelationEffect::RequireApproval => {
                record.context.extend(candidate);
                record
                    .context
                    .retain_recent(CAPABILITY_CONTEXT_OBSERVATION_LIMIT);
                record.updated_at = chrono::Utc::now();
                drop(contexts);
                self.record_capability_correlation_decision(
                    action_name,
                    &context_key,
                    "approval_disabled",
                    &decision,
                )
                .await;
                None
            }
        }
    }

    pub(in crate::runtime) async fn record_capability_correlation_decision(
        &self,
        action_name: &str,
        context_key: &str,
        outcome: &str,
        decision: &crate::security::capabilities::CapabilityCorrelationDecision,
    ) {
        let Some(report) = decision.report.as_ref() else {
            return;
        };
        let rules = report
            .matched_rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let subjects = report
            .observations
            .iter()
            .map(|observation| format!("{}:{}", observation.layer, observation.entity_id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", ");
        let severity = if matches!(
            decision.effect,
            crate::security::capabilities::CapabilityCorrelationEffect::Block
        ) {
            "high"
        } else {
            "medium"
        };
        self.record_security_event(
            "capability_correlation",
            severity,
            format!(
            "Runtime capability correlation: outcome={}, action='{}', rules=[{}], subjects=[{}]",
            outcome, action_name, rules, subjects
        ),
            Some(format!(
                "scope=runtime;context={};action={}",
                Self::truncate_audit_text(context_key, 128),
                action_name
            )),
        )
        .await;
    }

    pub async fn authorize_action_invocation(
        &self,
        action_name: &str,
        action_def: Option<&ActionDef>,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<ActionAuthorizationDecision> {
        let authorization = action_def
            .map(Self::merged_authorization_for_action)
            .unwrap_or_default();

        if let Some(decision) = self
            .authorize_action_scope(action_name, arguments, auth_context)
            .await
        {
            self.log_authorization_audit(
                action_name,
                arguments,
                &authorization,
                auth_context,
                &decision,
            )
            .await;
            return Ok(decision);
        }

        let decision = match auth_context.surface {
            ActionExecutionSurface::Internal | ActionExecutionSurface::Test => {
                ActionAuthorizationDecision::allow(
                    "Internal execution bypassed the interactive permission gate.",
                )
            }
            _ if authorization.human_approval.required
                && !auth_context.current_turn_is_explicit_approval =>
            {
                ActionAuthorizationDecision::allow(format!(
                    "Tool '{}' is allowed because interactive approval gates are disabled.",
                    action_name
                ))
            }
            _ if Self::direct_trusted_chat_tool_override(auth_context) => {
                ActionAuthorizationDecision::allow(format!(
                    "Tool '{}' is allowed because this is a direct authenticated chat request.",
                    action_name
                ))
            }
            _ if auth_context.direct_user_intent
                && matches!(
                    auth_context.surface,
                    ActionExecutionSurface::Chat | ActionExecutionSurface::Api
                )
                && auth_context
                    .principal
                    .as_ref()
                    .is_some_and(|principal| principal.trusted) =>
            {
                ActionAuthorizationDecision::allow(format!(
                    "Tool '{}' is allowed because this is a direct authenticated user request.",
                    action_name
                ))
            }
            _ if Self::is_background_surface(&auth_context.surface)
                && auth_context.direct_user_intent
                && auth_context
                    .principal
                    .as_ref()
                    .is_some_and(|principal| principal.trusted) =>
            {
                ActionAuthorizationDecision::allow(format!(
                    "Tool '{}' is allowed because this automation originated from a direct authenticated user request.",
                    action_name
                ))
            }
            _ if Self::is_background_surface(&auth_context.surface)
                && Self::risk_rank(&authorization.risk_level)
                    >= Self::risk_rank(&ActionRiskLevel::High) =>
            {
                ActionAuthorizationDecision::deny(format!(
                    "Tool '{}' is blocked in background or automation runs. Start it from a direct authenticated chat or API request instead.",
                    action_name
                ))
            }
            _ if authorization.requires_auth
                && !auth_context
                    .principal
                    .as_ref()
                    .is_some_and(|principal| principal.trusted) =>
            {
                ActionAuthorizationDecision::deny(format!(
                    "Tool '{}' requires a trusted local session. Run it from the authenticated UI or API instead of a background or anonymous context.",
                    action_name
                ))
            }
            _ if !authorization.allowed_roles.is_empty() => {
                let Some(principal) = auth_context.principal.as_ref() else {
                    let decision = ActionAuthorizationDecision::deny(format!(
                        "Tool '{}' requires an authorized local session with role access.",
                        action_name
                    ));
                    self.log_authorization_audit(
                        action_name,
                        arguments,
                        &authorization,
                        auth_context,
                        &decision,
                    )
                    .await;
                    return Ok(decision);
                };
                let matched_role = authorization
                    .allowed_roles
                    .iter()
                    .find(|role| role.eq_ignore_ascii_case(principal.role.as_str()))
                    .cloned();
                if let Some(role) = matched_role {
                    let mut decision = ActionAuthorizationDecision::allow(format!(
                        "Tool '{}' is allowed for the current trusted local session.",
                        action_name
                    ));
                    decision.matched_role = Some(role);
                    decision
                } else {
                    ActionAuthorizationDecision::deny(format!(
                        "Tool '{}' is not allowed for the current local session role '{}'.",
                        action_name, principal.role
                    ))
                }
            }
            _ => ActionAuthorizationDecision::allow(format!(
                "Tool '{}' is allowed for this request.",
                action_name
            )),
        };

        self.log_authorization_audit(
            action_name,
            arguments,
            &authorization,
            auth_context,
            &decision,
        )
        .await;
        if !decision.allowed {
            return Ok(decision);
        }

        if let Some(action_def) = action_def {
            if let Some(capability_decision) = self
                .authorize_capability_correlation(action_name, action_def, arguments, auth_context)
                .await
            {
                self.log_authorization_audit(
                    action_name,
                    arguments,
                    &authorization,
                    auth_context,
                    &capability_decision,
                )
                .await;
                return Ok(capability_decision);
            }

            let unapproved_permissions = self
                .unapproved_permissions_for_action(action_def, arguments, auth_context)
                .await;
            if !unapproved_permissions.is_empty() {
                let denied = ActionAuthorizationDecision::require_explicit_approval(
                    Self::build_permission_requirement_error(action_name, &unapproved_permissions),
                );
                self.log_authorization_audit(
                    action_name,
                    arguments,
                    &authorization,
                    auth_context,
                    &denied,
                )
                .await;
                return Ok(denied);
            }
        }

        Ok(decision)
    }
}
