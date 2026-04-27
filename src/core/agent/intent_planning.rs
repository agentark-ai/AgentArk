use super::*;

const INTENT_PLAN_TIMEOUT_MS: u64 = 90_000;
const INTENT_PLAN_MAX_ACTIONS: usize = 80;
const INTENT_PLAN_MAX_HISTORY: usize = 6;
const INTENT_PLAN_MAX_ENTITIES: usize = 36;
const INTENT_KIND_ANSWER: &str = "answer";
const INTENT_KIND_ACT: &str = "act";
const INTENT_KIND_DELEGATE: &str = "delegate";
const INTENT_KIND_INTERACTIVE: &str = "interactive";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdvisoryIntentPlan {
    #[serde(default)]
    pub intents: Vec<AdvisoryIntent>,
    #[serde(default)]
    pub is_conversational_only: bool,
    #[serde(default)]
    pub chain_relationship: String,
    #[serde(default)]
    pub rationale: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdvisoryIntent {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub likely_actions: Vec<String>,
    #[serde(default)]
    pub durability: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub qualifiers: AdvisoryIntentQualifiers,
    #[serde(default)]
    pub target_entity: Option<serde_json::Value>,
    #[serde(default)]
    pub delivery_channel: Option<String>,
    #[serde(default)]
    pub time_qualifier: Option<serde_json::Value>,
    #[serde(default)]
    pub source: Option<serde_json::Value>,
    #[serde(default)]
    pub requires_user_confirmation: bool,
    #[serde(default)]
    pub rationale: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdvisoryIntentQualifiers {
    #[serde(default)]
    pub target_entity: Option<serde_json::Value>,
    #[serde(default)]
    pub delivery_channel: Option<String>,
    #[serde(default)]
    pub time: Option<serde_json::Value>,
    #[serde(default)]
    pub source: Option<serde_json::Value>,
    #[serde(default)]
    pub inspect_target: Option<String>,
    #[serde(default)]
    pub extras: serde_json::Value,
}

impl AdvisoryIntent {
    pub fn qualifier_target_entity(&self) -> Option<&serde_json::Value> {
        self.qualifiers
            .target_entity
            .as_ref()
            .or(self.target_entity.as_ref())
    }

    pub fn qualifier_delivery_channel(&self) -> Option<&str> {
        self.qualifiers
            .delivery_channel
            .as_deref()
            .or(self.delivery_channel.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn qualifier_time(&self) -> Option<&serde_json::Value> {
        self.qualifiers
            .time
            .as_ref()
            .or(self.time_qualifier.as_ref())
    }

    pub fn qualifier_source(&self) -> Option<&serde_json::Value> {
        self.qualifiers.source.as_ref().or(self.source.as_ref())
    }

    pub fn qualifier_inspect_target(&self) -> Option<&str> {
        self.qualifiers
            .inspect_target
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

impl AdvisoryIntentPlan {
    pub fn likely_action_names(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for intent in &self.intents {
            for name in &intent.likely_actions {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if seen.insert(trimmed.to_string()) {
                    out.push(trimmed.to_string());
                }
            }
        }
        out
    }

    pub fn scope_query_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for intent in &self.intents {
            for value in [
                intent.kind.as_str(),
                intent.summary.as_str(),
                intent.durability.as_str(),
                intent.rationale.as_str(),
            ] {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    lines.push(trimmed.to_string());
                }
            }
            lines.extend(intent.likely_actions.iter().cloned());
            lines.extend(intent.depends_on.iter().map(|value| format!("depends_on {value}")));
            if let Some(channel) = intent.qualifier_delivery_channel() {
                lines.push(format!("delivery channel {channel}"));
            }
            if let Some(target) = intent.qualifier_target_entity() {
                lines.push(format!("target entity {}", safe_truncate(&target.to_string(), 260)));
            }
            if let Some(time) = intent.qualifier_time() {
                lines.push(format!("time qualifier {}", safe_truncate(&time.to_string(), 260)));
            }
            if let Some(source) = intent.qualifier_source() {
                lines.push(format!("source {}", safe_truncate(&source.to_string(), 260)));
            }
            if let Some(inspect_target) = intent.qualifier_inspect_target() {
                lines.push(format!("inspect target {inspect_target}"));
            }
            if !intent.qualifiers.extras.is_null() {
                lines.push(format!(
                    "extra qualifiers {}",
                    safe_truncate(&intent.qualifiers.extras.to_string(), 260)
                ));
            }
        }
        lines
    }
}

fn find_json_object_bounds(raw: &str) -> Option<(usize, usize)> {
    let bytes = raw.as_bytes();
    let start = bytes.iter().position(|b| *b == b'{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (idx, ch) in raw[start..].char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' && in_string {
            escape = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((start, start + idx + ch.len_utf8()));
                }
            }
            _ => {}
        }
    }
    None
}

fn extract_json_object(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if value.is_object() {
            return Some(value);
        }
    }
    let (start, end) = find_json_object_bounds(trimmed)?;
    serde_json::from_str::<serde_json::Value>(&trimmed[start..end]).ok()
}

fn normalize_intent_kind(raw: &str, has_likely_actions: bool) -> String {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        INTENT_KIND_ANSWER | INTENT_KIND_ACT | INTENT_KIND_DELEGATE | INTENT_KIND_INTERACTIVE => {
            normalized
        }
        _ if has_likely_actions => INTENT_KIND_ACT.to_string(),
        _ => INTENT_KIND_ANSWER.to_string(),
    }
}

fn normalize_chain_relationship(raw: &str, intent_count: usize) -> String {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "none" | "sequence" | "parallel" | "either" => normalized,
        _ if intent_count > 1 => "sequence".to_string(),
        _ => "none".to_string(),
    }
}

fn normalize_intent_durability(raw: &str, has_likely_actions: bool) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "ephemeral" | "none" => "ephemeral".to_string(),
        "session" => "session".to_string(),
        "persistent" => "persistent".to_string(),
        _ if has_likely_actions => "persistent".to_string(),
        _ => "ephemeral".to_string(),
    }
}

fn normalize_plan(mut plan: AdvisoryIntentPlan, authorized_actions: &HashSet<String>) -> AdvisoryIntentPlan {
    plan.intents.retain(|intent| {
        !intent.summary.trim().is_empty()
            || !intent.kind.trim().is_empty()
            || !intent.likely_actions.is_empty()
    });
    for (idx, intent) in plan.intents.iter_mut().enumerate() {
        if intent.id.trim().is_empty() {
            intent.id = format!("i{}", idx + 1);
        }
        intent.id = safe_truncate(intent.id.trim(), 48);
        intent.summary = safe_truncate(intent.summary.trim(), 220);
        intent.rationale = safe_truncate(intent.rationale.trim(), 260);
        intent.likely_actions = intent
            .likely_actions
            .iter()
            .map(|name| name.trim().to_string())
            .filter(|name| authorized_actions.contains(name))
            .take(5)
            .collect();
        intent.kind = normalize_intent_kind(&intent.kind, !intent.likely_actions.is_empty());
        intent.durability =
            normalize_intent_durability(&intent.durability, !intent.likely_actions.is_empty());
        intent.depends_on = intent
            .depends_on
            .iter()
            .map(|value| safe_truncate(value.trim(), 48))
            .filter(|value| !value.is_empty())
            .take(8)
            .collect();
        intent.delivery_channel = intent
            .delivery_channel
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| safe_truncate(value, 80));
        intent.qualifiers.delivery_channel = intent
            .qualifiers
            .delivery_channel
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| safe_truncate(value, 80));
        intent.qualifiers.inspect_target = intent
            .qualifiers
            .inspect_target
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| safe_truncate(value, 80));
    }
    plan.chain_relationship = normalize_chain_relationship(&plan.chain_relationship, plan.intents.len());
    plan.rationale = safe_truncate(plan.rationale.trim(), 300);
    if plan.intents.is_empty() {
        plan.is_conversational_only = true;
    }
    plan
}

fn compact_action_for_plan(action: &crate::actions::ActionDef) -> serde_json::Value {
    let metadata = action.planner_metadata();
    serde_json::json!({
        "name": action.name,
        "description": safe_truncate(&action.description, 260),
        "capabilities": action.capabilities,
        "metadata": {
            "role": metadata.role,
            "delivery_mode": metadata.delivery_mode,
            "integration_class": metadata.integration_class,
            "side_effect_level": metadata.side_effect_level,
            "requires_auth": metadata.requires_auth,
        },
    })
}

fn pending_action_entity(action: &PendingConversationAction) -> serde_json::Value {
    serde_json::json!({
        "kind": action.kind.as_router_kind(),
        "id": action.key,
        "summary": safe_truncate(&crate::security::redact_secret_input(&action.summary).text, 240),
    })
}

fn background_session_entity(session: &crate::core::background_session::BackgroundSession) -> serde_json::Value {
    serde_json::json!({
        "kind": "background_session",
        "id": session.id,
        "title": safe_truncate(&crate::security::redact_secret_input(&session.title).text, 160),
        "summary": session.summary.as_ref().map(|value| {
            safe_truncate(&crate::security::redact_secret_input(value).text, 220)
        }),
        "status": session.status.label(),
        "updated_at": session.updated_at,
    })
}

fn watcher_entity(watcher: &crate::core::watcher::Watcher) -> serde_json::Value {
    serde_json::json!({
        "kind": "watcher",
        "id": watcher.id.to_string(),
        "summary": safe_truncate(&crate::security::redact_secret_input(&watcher.description).text, 240),
        "status": serde_json::to_value(&watcher.status).unwrap_or_else(|_| {
            serde_json::Value::String(format!("{:?}", watcher.status))
        }),
        "updated_at": watcher.last_poll_at,
    })
}

fn app_entity(app: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "kind": "app",
        "id": app.get("id").and_then(|value| value.as_str()).unwrap_or_default(),
        "title": app.get("title").and_then(|value| value.as_str()).unwrap_or("App"),
        "url": app.get("url").or_else(|| app.get("access_url")).cloned(),
        "running": app.get("running").and_then(|value| value.as_bool()),
        "updated_at": app.get("last_accessed").or_else(|| app.get("updated_at")).cloned(),
    })
}

fn intent_plan_system_prompt() -> &'static str {
    "You are AgentArk's advisory intent planner. Produce a compact JSON plan that decomposes the user's underlying requested outcomes into broad kinds: answer, act, delegate, or interactive. Put specific details in qualifiers and likely_actions, not in new intent kinds. This plan is advisory only: the execution loop will choose tools. Do not block, refuse, or answer the user. Do not depend on keywords or phrasing; infer meaning from the whole request, conversation context, action catalog, and recent entities. Prefer action names only from the provided catalog when likely_actions are useful. Return strict JSON only."
}

fn intent_plan_prompt(
    message: &str,
    packed_context: &super::conversation_context::PackedConversationContext,
    pending_actions: &[PendingConversationAction],
    background_sessions: &[crate::core::background_session::BackgroundSession],
    watchers: &[crate::core::watcher::Watcher],
    apps: &[serde_json::Value],
    authorized_actions: &[crate::actions::ActionDef],
) -> String {
    let recent_messages = packed_context
        .history
        .iter()
        .rev()
        .take(INTENT_PLAN_MAX_HISTORY)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|turn| {
            serde_json::json!({
                "role": turn.role,
                "content": safe_truncate(
                    &crate::security::redact_secret_input(&turn.content).text,
                    700,
                ),
                "timestamp": turn._timestamp,
            })
        })
        .collect::<Vec<_>>();
    let mut recent_entities = Vec::new();
    recent_entities.extend(pending_actions.iter().take(8).map(pending_action_entity));
    recent_entities.extend(background_sessions.iter().take(10).map(background_session_entity));
    recent_entities.extend(watchers.iter().take(10).map(watcher_entity));
    recent_entities.extend(apps.iter().take(8).map(app_entity));
    recent_entities.truncate(INTENT_PLAN_MAX_ENTITIES);

    let actions = authorized_actions
        .iter()
        .take(INTENT_PLAN_MAX_ACTIONS)
        .map(compact_action_for_plan)
        .collect::<Vec<_>>();

    serde_json::json!({
        "user_message": message,
        "conversation": {
            "recent_messages": recent_messages,
            "earlier_recap": packed_context.digest.as_ref().map(|value| safe_truncate(value, 1400)),
        },
        "recent_entities": recent_entities,
        "authorized_action_catalog": actions,
        "required_output_schema": {
            "intents": [{
                "id": "i1",
                "kind": "answer | act | delegate | interactive",
                "summary": "one sentence describing the user-visible outcome",
                "likely_actions": ["0 to 5 action names from authorized_action_catalog"],
                "durability": "ephemeral | session | persistent",
                "depends_on": ["ids of prerequisite intents"],
                "qualifiers": {
                    "target_entity": {"kind": "optional recent entity kind", "id": "optional entity id"},
                    "delivery_channel": "optional channel target",
                    "time": {"optional": "timing or recurrence details"},
                    "source": {"optional": "repo/url/file/rtsp/integration/source details"},
                    "inspect_target": "optional live internal surface or external target to inspect",
                    "extras": {"optional": "open structured qualifiers that do not fit the common fields"}
                },
                "requires_user_confirmation": false,
                "rationale": "brief reason this decomposition/action hint fits"
            }],
            "is_conversational_only": false,
            "chain_relationship": "none, sequence, parallel, either",
            "rationale": "brief plan-level rationale"
        },
        "rules": [
            "The plan never blocks execution.",
            "For live/current product state, prefer live inspection/search actions over documentation actions.",
            "For product how-to or capability explanation, product documentation actions may be likely.",
            "For generated runnable apps/sites/dashboards/tools, include the app-hosting/deployment action.",
            "For monitoring/recurrence/reminders/watch-until-condition work, preserve timing and delivery qualifiers.",
            "Use kind=act for concrete actions including deploy, schedule, watch, modify, install, query live state, research, and integration work.",
            "Use kind=answer for textual responses, even when a read-only retrieval action may help.",
            "Use kind=delegate only when work should be split across sub-agents.",
            "Use kind=interactive only when explicit user checkpoints or a resumable browser/login/session flow are required.",
            "For mixed requests, create one intent per user-visible outcome even when the outcomes use different actions or one outcome is deploy/write-oriented while another is live-state/query-oriented.",
            "For multi-outcome requests, create multiple intents and dependency edges instead of collapsing them."
        ]
    })
    .to_string()
}

impl Agent {
    pub(super) async fn build_advisory_intent_plan(
        &self,
        message: &str,
        packed_context: &super::conversation_context::PackedConversationContext,
        pending_actions: &[PendingConversationAction],
        background_sessions: &[crate::core::background_session::BackgroundSession],
        watchers: &[crate::core::watcher::Watcher],
        authorized_actions: &[crate::actions::ActionDef],
    ) -> Option<AdvisoryIntentPlan> {
        if message.trim().is_empty() {
            return None;
        }
        let apps = self.app_registry.list().await;
        let prompt = intent_plan_prompt(
            message,
            packed_context,
            pending_actions,
            background_sessions,
            watchers,
            &apps,
            authorized_actions,
        );
        let response = self
            .supervised_internal_chat(
                "automation",
                "intent_plan",
                "advisory_intent_plan",
                &ModelRole::Primary,
                self.llm_candidates_for_role(&ModelRole::Primary),
                intent_plan_system_prompt(),
                &prompt,
                &[],
                &[],
                INTENT_PLAN_TIMEOUT_MS,
                2,
            )
            .await?;
        let value = extract_json_object(&response.content)?;
        let authorized_names = authorized_actions
            .iter()
            .map(|action| action.name.clone())
            .collect::<HashSet<_>>();
        serde_json::from_value::<AdvisoryIntentPlan>(value)
            .ok()
            .map(|plan| normalize_plan(plan, &authorized_names))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_plan_collapses_specific_action_kind_to_broad_act() {
        let authorized = HashSet::from(["app_deploy".to_string()]);
        let plan = AdvisoryIntentPlan {
            intents: vec![AdvisoryIntent {
                id: String::new(),
                kind: "app_deploy".to_string(),
                summary: "Create a runnable app".to_string(),
                likely_actions: vec!["app_deploy".to_string()],
                durability: "deployment".to_string(),
                ..AdvisoryIntent::default()
            }],
            chain_relationship: "custom".to_string(),
            ..AdvisoryIntentPlan::default()
        };

        let normalized = normalize_plan(plan, &authorized);

        assert_eq!(normalized.intents[0].kind, INTENT_KIND_ACT);
        assert_eq!(
            normalized.intents[0].likely_actions,
            vec!["app_deploy".to_string()]
        );
        assert_eq!(normalized.chain_relationship, "none");
    }

    #[test]
    fn normalize_plan_preserves_qualifiers_and_four_structural_kinds() {
        let authorized = HashSet::from(["browser_auto".to_string()]);
        let plan = AdvisoryIntentPlan {
            intents: vec![AdvisoryIntent {
                id: "flow".to_string(),
                kind: "interactive".to_string(),
                summary: "Use a browser flow with a checkpoint".to_string(),
                likely_actions: vec!["browser_auto".to_string()],
                qualifiers: AdvisoryIntentQualifiers {
                    inspect_target: Some("trace".to_string()),
                    extras: serde_json::json!({"checkpoint": "user_login"}),
                    ..AdvisoryIntentQualifiers::default()
                },
                ..AdvisoryIntent::default()
            }],
            ..AdvisoryIntentPlan::default()
        };

        let normalized = normalize_plan(plan, &authorized);

        assert_eq!(normalized.intents[0].kind, INTENT_KIND_INTERACTIVE);
        assert_eq!(
            normalized.intents[0].qualifier_inspect_target(),
            Some("trace")
        );
        assert_eq!(
            normalized.intents[0].qualifiers.extras["checkpoint"],
            serde_json::json!("user_login")
        );
    }
}
