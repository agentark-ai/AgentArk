use super::*;

const DIRECT_CONVERSATION_VERSION: &str = "direct_conversation_v1";
const DIRECT_CONTEXT_MODEL_USED: &str = "direct_context";
const DIRECT_MEMORY_MODEL_USED: &str = "direct_memory";
const DIRECT_CONVERSATION_MODEL_USED: &str = "direct_conversation";
const DIRECT_CONVERSATION_TIMEOUT_MS: u64 = 30_000;
const DIRECT_CONVERSATION_MAX_CANDIDATES: usize = 2;
const DIRECT_CONVERSATION_RECENT_MESSAGES: usize = 6;
const DIRECT_CONVERSATION_RECENT_ARTIFACTS: usize = 3;
const INBOUND_CLASSIFIER_RECENT_ARTIFACTS: usize = 4;
const DIRECT_MEMORY_MAX_ITEMS: u64 = 24;
const DIRECT_MEMORY_MAX_LIST_ITEMS: usize = 5;
const DIRECT_LOCAL_ANSWER_MIN_SCORE: f32 = 0.70;
const DIRECT_LOCAL_ANSWER_MIN_MARGIN: f32 = 0.04;
const DEFERRED_CHAT_PERSISTENCE_MAX_CONCURRENCY: usize = 8;
const DEFERRED_CHAT_PERSISTENCE_ATTEMPTS: usize = 3;
const DEFERRED_CHAT_PERSISTENCE_ATTEMPT_TIMEOUT_SECS: u64 = 45;
const DEFERRED_CHAT_PERSISTENCE_WARN_PENDING: usize = 64;

static DEFERRED_CHAT_PERSISTENCE_SEMAPHORE: once_cell::sync::Lazy<Arc<tokio::sync::Semaphore>> =
    once_cell::sync::Lazy::new(|| {
        Arc::new(tokio::sync::Semaphore::new(
            DEFERRED_CHAT_PERSISTENCE_MAX_CONCURRENCY,
        ))
    });
static DEFERRED_CHAT_PERSISTENCE_PENDING: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Default)]
struct DirectConversationRuntimeState {
    routing_trusted: bool,
    has_attachments: bool,
    has_secret_offered: bool,
    has_pending_actions: bool,
    has_pending_credential_prompt: bool,
    user_message_already_recorded: bool,
    skip_inbound_security_precheck: bool,
    supported_surface: bool,
}

fn attachment_hint_is_visual(attachment: &ChatAttachmentHint) -> bool {
    let kind = attachment.kind.trim().to_ascii_lowercase();
    let content_type = attachment
        .content_type
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    kind.contains("visual") || kind.contains("image") || content_type.starts_with("image/")
}

fn truncate_for_attachment_memory_source(value: &str, max_chars: usize) -> String {
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
}

fn attachment_only_visual_memory_capture_source(
    message: &str,
    response: &str,
    request_hints: &RequestExecutionHints,
) -> Option<String> {
    if !message.trim().is_empty() {
        return None;
    }
    if !request_hints
        .attachments
        .iter()
        .any(attachment_hint_is_visual)
    {
        return None;
    }
    let response = response.trim();
    if response.is_empty() {
        return None;
    }

    let visual_analysis = truncate_for_attachment_memory_source(response, 1_100);
    Some(format!(
        "Memory extraction input for a visual-only user turn. Analyze only durable user preferences or reusable workflow constraints that are supported by the visual analysis below. Do not store that the user sent an attachment or omitted text. Do not store one-off image contents, identities, sensitive traits, credentials, or guesses.\n\nVisual analysis:\n{}",
        visual_analysis
    ))
}

fn redact_chat_message_for_storage(secret_scrubbed_message: &str) -> String {
    let mut redactor = crate::security::pii::PiiRedactor::new();
    redactor.redact_emails = false;
    redactor.redact_phones = false;
    redactor.redact_ips = false;
    redactor.redact(secret_scrubbed_message)
}

fn has_contact_info_for_memory_capture(secret_scrubbed_message: &str) -> bool {
    let mut redactor = crate::security::pii::PiiRedactor::new();
    redactor.redact_ssn = false;
    redactor.redact_credit_cards = false;
    redactor.redact_ips = false;
    redactor.redact(secret_scrubbed_message) != secret_scrubbed_message
}

#[derive(Debug, serde::Deserialize)]
struct DirectConversationModelOutput {
    #[serde(default)]
    can_answer_directly: bool,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    decline_kind: Option<DirectConversationDeclineKind>,
    #[serde(default)]
    rationale: Option<String>,
}

#[derive(Clone, Copy, Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DirectConversationDeclineKind {
    ExternalInfo,
    LiveState,
    ProductHelp,
    SavedUserFacts,
    ArtifactOrFile,
    MutationOrDurable,
    MissingContext,
    UnsafeOrAuth,
    #[serde(other)]
    Unknown,
}

#[derive(Debug)]
enum DirectConversationResponse {
    Answer(String),
    Declined {
        kind: Option<DirectConversationDeclineKind>,
        rationale: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TurnExecutionPath {
    DirectReply,
    AgentLoop,
}

fn turn_execution_path_from_routing(
    routing: Option<&crate::security::intent_classifier::InboundRoutingSignal>,
    state: DirectConversationRuntimeState,
) -> TurnExecutionPath {
    if should_use_direct_conversation_path(routing, state) {
        TurnExecutionPath::DirectReply
    } else {
        TurnExecutionPath::AgentLoop
    }
}

fn neutralize_direct_reply_routing_after_direct_decline(
    routing: Option<&mut crate::security::intent_classifier::InboundRoutingSignal>,
    state: DirectConversationRuntimeState,
    decline_kind: Option<DirectConversationDeclineKind>,
    original_user_message: &str,
    decline_rationale: Option<&str>,
) -> bool {
    let Some(signal) = routing else {
        return false;
    };
    if turn_execution_path_from_routing(Some(&*signal), state) != TurnExecutionPath::DirectReply {
        return false;
    }

    signal.should_execute = true;
    signal.tool_use_expected = true;
    signal.semantic_queries.clear();
    signal.required_capabilities.clear();

    let compact_user_message = original_user_message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let compact_user_message = compact_user_message.trim();
    let build_decline_query = |capability: &str| -> String {
        if compact_user_message.is_empty() {
            capability.to_string()
        } else {
            format!(
                "{} for current user request: {}",
                capability,
                safe_truncate(compact_user_message, 260)
            )
        }
    };
    let add_decline_query =
        |signal: &mut crate::security::intent_classifier::InboundRoutingSignal,
         capability: &str| {
            let query = build_decline_query(capability);
            signal.semantic_queries.push(safe_truncate(&query, 360));
            signal
                .required_capabilities
                .push(safe_truncate(&query, 220));
            query
        };
    let set_decline_goal =
        |signal: &mut crate::security::intent_classifier::InboundRoutingSignal,
         capability: &str,
         expected_outcome: &str,
         durability: &str,
         groundings: Vec<String>,
         side_effect: &str| {
            let query = add_decline_query(signal, capability);
            signal.goals = vec![crate::security::intent_classifier::InboundTurnGoal {
                id: "g1".to_string(),
                intent_summary: safe_truncate(capability, 160),
                capability_query: safe_truncate(&query, 180),
                expected_outcome: safe_truncate(expected_outcome, 180),
                durability: durability.to_string(),
                groundings,
                side_effect: side_effect.to_string(),
                dependencies: Vec::new(),
            }];
        };

    match decline_kind.unwrap_or(DirectConversationDeclineKind::Unknown) {
        DirectConversationDeclineKind::ExternalInfo => {
            signal.current_answer_expected = true;
            signal.external_info_expected = true;
            set_decline_goal(
                signal,
                "external public information lookup",
                "A grounded answer from public external information",
                "none",
                vec!["external_info".to_string()],
                "none",
            );
        }
        DirectConversationDeclineKind::LiveState
        | DirectConversationDeclineKind::ArtifactOrFile => {
            signal.current_answer_expected = true;
            signal.live_state_expected = true;
            set_decline_goal(
                signal,
                "live local state or artifact inspection",
                "A grounded answer from current local state or artifacts",
                "none",
                vec!["local_state".to_string()],
                "none",
            );
        }
        DirectConversationDeclineKind::ProductHelp => {
            signal.current_answer_expected = true;
            signal.product_help_expected = true;
            set_decline_goal(
                signal,
                "product documentation or capability lookup",
                "A grounded product-help answer",
                "none",
                vec!["product_help".to_string()],
                "none",
            );
        }
        DirectConversationDeclineKind::SavedUserFacts => {
            signal.current_answer_expected = true;
            signal.saved_user_facts_expected = true;
            set_decline_goal(
                signal,
                "saved user fact or preference lookup",
                "A grounded answer from saved user facts or preferences",
                "none",
                vec!["user_memory".to_string()],
                "none",
            );
        }
        DirectConversationDeclineKind::MutationOrDurable => {
            signal.current_answer_expected = true;
            signal.durable_work_expected = true;
            set_decline_goal(
                signal,
                "durable action planning",
                "The requested durable action is planned or executed",
                "persistent_work",
                Vec::new(),
                "write",
            );
        }
        DirectConversationDeclineKind::MissingContext
        | DirectConversationDeclineKind::UnsafeOrAuth
        | DirectConversationDeclineKind::Unknown => {
            signal.current_answer_expected = true;
            if !compact_user_message.is_empty() {
                signal
                    .semantic_queries
                    .push(safe_truncate(compact_user_message, 360));
            }
            signal.goals = vec![crate::security::intent_classifier::InboundTurnGoal {
                id: "g1".to_string(),
                intent_summary: "Route current request through execution planning".to_string(),
                capability_query: safe_truncate(compact_user_message, 180),
                expected_outcome: "The current request is handled outside the direct reply path"
                    .to_string(),
                durability: "none".to_string(),
                groundings: Vec::new(),
                side_effect: "write".to_string(),
                dependencies: Vec::new(),
            }];
        }
    }
    signal.multi_goal = signal.has_multiple_goals();
    signal.durable_work_expected = signal.has_durable_goal();
    signal.tool_use_expected = signal.has_executable_goal();
    signal.should_execute = signal.tool_use_expected;
    signal.rationale = Some(
        match decline_rationale
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(rationale) => format!(
                "Direct response path semantically declined: {}; route through execution planning.",
                safe_truncate(rationale, 180)
            ),
            None => "Direct response path semantically declined; route through execution planning."
                .to_string(),
        },
    );
    true
}

fn direct_runtime_state_allows_immediate_reply(state: DirectConversationRuntimeState) -> bool {
    state.routing_trusted
        && !state.has_attachments
        && !state.has_secret_offered
        && !state.has_pending_actions
        && !state.has_pending_credential_prompt
        && !state.user_message_already_recorded
        && !state.skip_inbound_security_precheck
        && state.supported_surface
}

fn should_enqueue_semantic_user_memory_capture(
    message: &str,
    state: DirectConversationRuntimeState,
    turn_path: TurnExecutionPath,
) -> bool {
    let _ = (message, state, turn_path);
    false
}

fn routing_is_transient_read_only_lookup(
    routing: Option<&crate::security::intent_classifier::InboundRoutingSignal>,
) -> bool {
    let Some(signal) = routing else {
        return false;
    };
    signal.has_transient_read_only_lookup()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectMemoryLookupKind {
    Identity,
    Location,
    Timezone,
    Preference,
    Contact,
    Constraint,
    Any,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectLocalAnswerMode {
    PreviousUserMessage,
    PreviousAssistantMessage,
    RecentConversationSummary,
}

impl DirectLocalAnswerMode {
    fn canonical_text(self) -> &'static str {
        match self {
            Self::PreviousUserMessage => {
                "The user asks to recall, quote, restate, identify, or answer with the immediately previous user message or question from this conversation."
            }
            Self::PreviousAssistantMessage => {
                "The user asks to recall, quote, restate, identify, or answer with the assistant's immediately previous response from this conversation."
            }
            Self::RecentConversationSummary => {
                "The user asks for a concise summary, recap, or description of the recent visible conversation history."
            }
        }
    }
}

fn direct_local_answer_mode_query(
    routing: &crate::security::intent_classifier::InboundRoutingSignal,
) -> Option<String> {
    let mut parts = Vec::new();
    parts.extend(routing.semantic_queries.iter().map(String::as_str));
    for goal in &routing.goals {
        parts.push(goal.intent_summary.as_str());
        parts.push(goal.capability_query.as_str());
        parts.push(goal.expected_outcome.as_str());
    }
    let query = parts
        .into_iter()
        .map(|part| part.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!query.trim().is_empty()).then_some(query)
}

impl DirectMemoryLookupKind {
    fn from_routing_value(value: Option<&str>) -> Self {
        match value.unwrap_or_default().trim() {
            "identity" => Self::Identity,
            "location" => Self::Location,
            "timezone" => Self::Timezone,
            "preference" => Self::Preference,
            "contact" => Self::Contact,
            "constraint" => Self::Constraint,
            _ => Self::Any,
        }
    }

    fn as_memory_kind(self) -> Option<&'static str> {
        match self {
            Self::Identity => Some("identity"),
            Self::Location => Some("location"),
            Self::Timezone => Some("timezone"),
            Self::Preference => Some("preference"),
            Self::Contact => Some("contact"),
            Self::Constraint => Some("constraint"),
            Self::Any => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Location => "location",
            Self::Timezone => "timezone",
            Self::Preference => "preference",
            Self::Contact => "contact detail",
            Self::Constraint => "operating constraint",
            Self::Any => "saved fact",
        }
    }
}

#[derive(Clone, Debug)]
struct DirectMemoryCandidate {
    lookup_kind: DirectMemoryLookupKind,
    value: String,
    content: String,
    scope_rank: u8,
    confidence: f64,
    support_count: i32,
    updated_at: String,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
struct TurnPipelineUsageSnapshot {
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    cost_usd: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeferredExchangePersistenceKind {
    TurnPipeline,
    Immediate,
}

#[derive(Clone)]
struct DeferredExchangePersistence {
    kind: DeferredExchangePersistenceKind,
    trace_snapshot: ExecutionTrace,
    message: String,
    response: String,
    channel: String,
    conversation_key: String,
    project_id: Option<String>,
    model_used: String,
    user_message_already_recorded: bool,
    memory_capture_allowed: bool,
    memory_capture_source: Option<String>,
    user_message_id: String,
    assistant_message_id: String,
    user_timestamp: String,
    assistant_timestamp: String,
    is_new_conversation: bool,
    conversation_title: Option<String>,
    user_outcome: crate::core::UserFacingOutcome,
}

impl TurnPipelineUsageSnapshot {
    fn delta_since(self, previous: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_sub(previous.input_tokens),
            output_tokens: self.output_tokens.saturating_sub(previous.output_tokens),
            total_tokens: self.total_tokens.saturating_sub(previous.total_tokens),
            cost_usd: (self.cost_usd - previous.cost_usd).max(0.0),
        }
    }
}

fn memory_capture_source_with_completed_work_context(
    message: &str,
    response: &str,
    records: &[AgentTurnRecord],
    turn_plan: Option<&ExecutionPlan>,
) -> Option<String> {
    let completed_work = records
        .iter()
        .filter(|record| record.outcome == AgentTurnOutcomeKind::Succeeded)
        .filter(|record| {
            record
                .side_effect
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty() && value != "none")
                || record.resolved_object_ref.is_some()
        })
        .map(|record| {
            serde_json::json!({
                "goal_id": &record.goal_id,
                "action": record.action_name.as_ref(),
                "side_effect": record.side_effect.as_ref(),
                "object_ref": record.resolved_object_ref.as_ref(),
                "output": record.tool_output.as_ref().map(|value| {
                    safe_truncate(&value.to_string(), 900)
                }),
            })
        })
        .collect::<Vec<_>>();
    if completed_work.is_empty() {
        return None;
    }

    let plan_summary = turn_plan.map(|plan| {
        serde_json::json!({
            "summary": &plan.summary,
            "steps": plan.steps.iter().map(|step| {
                serde_json::json!({
                    "title": &step.title,
                    "description": &step.description,
                    "action": step.action.as_ref(),
                    "status": step.status.as_ref(),
                })
            }).collect::<Vec<_>>(),
        })
    });

    Some(format!(
        "Memory extraction input for a turn that also created or updated durable work objects.\n\
         The durable work records below already own their task-specific schedules, watcher conditions, notification routes, targets, execution state, and follow-up instructions. Do not store those object-specific details as ArkMemory. Only extract durable user facts, stable preferences, reusable constraints, or cross-context workflow rules that remain useful independently of these created work objects.\n\n\
         User message:\n{}\n\n\
         Assistant response:\n{}\n\n\
         Durable work created or updated this turn:\n{}\n\n\
         Turn plan:\n{}",
        safe_truncate(message, 1800),
        safe_truncate(response, 1400),
        serde_json::to_string_pretty(&completed_work).unwrap_or_default(),
        plan_summary
            .and_then(|value| serde_json::to_string_pretty(&value).ok())
            .unwrap_or_else(|| "null".to_string())
    ))
}

fn should_use_direct_conversation_path(
    routing: Option<&crate::security::intent_classifier::InboundRoutingSignal>,
    state: DirectConversationRuntimeState,
) -> bool {
    match routing {
        Some(signal) => direct_runtime_state_allows_immediate_reply(state)
            && signal.is_conversational_only(),
        None => {
            !state.has_attachments
                && !state.has_secret_offered
                && !state.has_pending_actions
                && !state.has_pending_credential_prompt
                && !state.user_message_already_recorded
                && !state.skip_inbound_security_precheck
                && state.supported_surface
        }
    }
}

fn direct_memory_scope_rank(
    item: &crate::storage::experience_item::Model,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
) -> u8 {
    let project_rank = if project_id.is_some() && item.project_id.as_deref() == project_id {
        1u8
    } else {
        0u8
    };
    let conversation_rank =
        if conversation_id.is_some() && item.conversation_id.as_deref() == conversation_id {
            2u8
        } else {
            0u8
        };
    project_rank + conversation_rank
}

fn direct_memory_value_key(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn direct_memory_candidate_from_item(
    item: &crate::storage::experience_item::Model,
    lookup_kind: DirectMemoryLookupKind,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<DirectMemoryCandidate> {
    if !should_inject_learned_user_memory(item, now) {
        return None;
    }
    let item_kind = learned_user_memory_lookup_kind(item);
    if let Some(required_kind) = lookup_kind.as_memory_kind() {
        if item_kind != required_kind {
            return None;
        }
    }
    let value = learned_user_memory_value(item)?;
    let value = crate::security::redact_secret_input(&value).text;
    let value = safe_truncate(value.trim(), 220);
    if value.is_empty() {
        return None;
    }
    let content = format_learned_user_memory_for_prompt(item, now)
        .unwrap_or_else(|| format!("- [{}] {}", lookup_kind.label(), safe_truncate(&value, 180)));
    Some(DirectMemoryCandidate {
        lookup_kind,
        value,
        content,
        scope_rank: direct_memory_scope_rank(item, project_id, conversation_id),
        confidence: item.confidence,
        support_count: item.support_count,
        updated_at: item.updated_at.clone(),
    })
}

fn sorted_direct_memory_candidates(
    items: &[crate::storage::experience_item::Model],
    lookup_kind: DirectMemoryLookupKind,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<DirectMemoryCandidate> {
    let mut candidates = items
        .iter()
        .filter_map(|item| {
            direct_memory_candidate_from_item(item, lookup_kind, project_id, conversation_id, now)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .scope_rank
            .cmp(&left.scope_rank)
            .then_with(|| right.confidence.total_cmp(&left.confidence))
            .then_with(|| right.support_count.cmp(&left.support_count))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
    });
    candidates
}

fn direct_memory_answer_from_candidates(
    candidates: &[DirectMemoryCandidate],
    lookup_kind: DirectMemoryLookupKind,
) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    if lookup_kind == DirectMemoryLookupKind::Any {
        let mut seen = HashSet::new();
        let facts = candidates
            .iter()
            .filter_map(|candidate| {
                let key = direct_memory_value_key(&candidate.content);
                seen.insert(key).then(|| candidate.content.clone())
            })
            .take(DIRECT_MEMORY_MAX_LIST_ITEMS)
            .collect::<Vec<_>>();
        return match facts.len() {
            0 => None,
            1 => Some(format!("I have this saved about you:\n{}", facts[0].trim())),
            _ => Some(format!(
                "I have these saved facts about you:\n{}",
                facts.join("\n")
            )),
        };
    }

    let best_scope_rank = candidates[0].scope_rank;
    let best = candidates
        .iter()
        .filter(|candidate| candidate.scope_rank == best_scope_rank)
        .collect::<Vec<_>>();
    let distinct_values = best
        .iter()
        .map(|candidate| direct_memory_value_key(&candidate.value))
        .collect::<HashSet<_>>();
    if distinct_values.len() != 1 {
        return None;
    }

    let value = best[0].value.trim();
    if value.is_empty() {
        return None;
    }
    Some(format!(
        "I have this saved {} for you: {}.",
        best[0].lookup_kind.label(),
        value
    ))
}

fn select_direct_memory_answer(
    items: &[crate::storage::experience_item::Model],
    profile_lookup_kind: Option<&str>,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<String> {
    let lookup_kind = DirectMemoryLookupKind::from_routing_value(profile_lookup_kind);
    let candidates =
        sorted_direct_memory_candidates(items, lookup_kind, project_id, conversation_id, now);
    direct_memory_answer_from_candidates(&candidates, lookup_kind)
}

fn extract_direct_conversation_json_object(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if value.is_object() {
            return Some(value);
        }
    }
    let start = trimmed
        .char_indices()
        .find_map(|(idx, ch)| if ch == '{' { Some(idx) } else { None })?;
    let end = trimmed.char_indices().rev().find_map(|(idx, ch)| {
        if ch == '}' {
            Some(idx + ch.len_utf8())
        } else {
            None
        }
    })?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(&trimmed[start..end])
        .ok()
        .filter(|value| value.is_object())
}

fn direct_conversation_system_prompt() -> String {
    format!(
        "You are {name}. This is a no-tool direct conversation path. \
Answer only from the user message, visible recent conversation, saved user facts, and product identity supplied in this prompt. \
Do not claim to inspect live state, files, tools, integrations, web pages, documents, apps, logs, clocks, or external systems. \
Do not state or imply persistence of newly supplied user facts unless they are already present in saved_user_facts; acknowledge new self-information plainly instead. \
If the request needs tool use, live/current/external information, mutation, scheduling, deployment, integration work, files, code execution, approvals, attachments, or missing context, set can_answer_directly=false instead of writing a refusal, and set decline_kind to the closest semantic class: external_info, live_state, product_help, saved_user_facts, artifact_or_file, mutation_or_durable, missing_context, unsafe_or_auth, or unknown. \
If recent_actionable_artifacts are supplied and the user is asking about, debugging, validating, fixing, changing, or continuing work on one of them, set can_answer_directly=false with decline_kind=artifact_or_file instead of pretending to inspect it. \
Return only compact JSON with this exact shape: {{\"can_answer_directly\":true,\"answer\":\"final user-facing response\",\"decline_kind\":null,\"rationale\":\"brief reason\"}} or {{\"can_answer_directly\":false,\"answer\":\"\",\"decline_kind\":\"external_info\",\"rationale\":\"brief reason\"}}. \
Do not mention routing, policies, classifiers, this direct path, or the full agent loop in the answer.",
        name = crate::branding::PRODUCT_NAME
    )
}

fn direct_conversation_user_prompt(
    message: &str,
    conversation_key: &str,
    recent_messages: &[serde_json::Value],
    recent_artifacts: &[serde_json::Value],
    saved_user_facts_context: Option<&str>,
) -> String {
    serde_json::json!({
        "conversation_id": conversation_key,
        "product_identity": {
            "name": crate::branding::PRODUCT_NAME,
        },
        "recent_messages": recent_messages,
        "recent_actionable_artifacts": recent_artifacts,
        "saved_user_facts": saved_user_facts_context,
        "user_message": message,
    })
    .to_string()
}

impl Agent {
    async fn direct_conversation_recent_messages(
        &self,
        conversation_key: &str,
    ) -> Vec<serde_json::Value> {
        let history = self.conversation_history.read().await;
        let Some(messages) = history.get(conversation_key) else {
            return Vec::new();
        };
        let mut recent = messages
            .iter()
            .rev()
            .take(DIRECT_CONVERSATION_RECENT_MESSAGES)
            .map(|message| {
                serde_json::json!({
                    "role": message.role.clone(),
                    "content": safe_truncate(
                        &crate::security::redact_secret_input(&message.content).text,
                        700,
                    ),
                    "timestamp": message._timestamp,
                })
            })
            .collect::<Vec<_>>();
        recent.reverse();
        recent
    }

    async fn classify_direct_local_answer_mode(
        &self,
        routing: &crate::security::intent_classifier::InboundRoutingSignal,
    ) -> Option<DirectLocalAnswerMode> {
        let embedder = self.embedding_client.as_deref()?;
        let query = direct_local_answer_mode_query(routing)?;
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        let modes = [
            DirectLocalAnswerMode::PreviousUserMessage,
            DirectLocalAnswerMode::PreviousAssistantMessage,
            DirectLocalAnswerMode::RecentConversationSummary,
        ];
        let mut texts = Vec::with_capacity(modes.len() + 1);
        texts.push(query.to_string());
        texts.extend(modes.iter().map(|mode| mode.canonical_text().to_string()));
        let embeddings = embedder.embed_texts(&texts).await.ok()?;
        let message_embedding = embeddings.first()?;
        let mut scored = modes
            .iter()
            .zip(embeddings.iter().skip(1))
            .filter_map(|(mode, embedding)| {
                let score = crate::core::document_search::normalized_embedding_similarity(
                    message_embedding.as_slice(),
                    embedding.as_slice(),
                )?;
                Some((*mode, score))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let (mode, score) = *scored.first()?;
        let competing = scored.get(1).map(|(_, score)| *score).unwrap_or(0.0);
        if score >= DIRECT_LOCAL_ANSWER_MIN_SCORE
            && score - competing >= DIRECT_LOCAL_ANSWER_MIN_MARGIN
        {
            Some(mode)
        } else {
            None
        }
    }

    async fn enrich_product_help_routing_doc_ids(
        &self,
        routing: &mut crate::security::intent_classifier::InboundRoutingSignal,
        message: &str,
    ) {
        if !routing.product_help_expected || !routing.grounding_doc_ids.is_empty() {
            return;
        }
        let query = routing
            .semantic_queries
            .iter()
            .find_map(|value| {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            })
            .unwrap_or_else(|| message.trim().to_string());
        if query.is_empty() {
            return;
        }
        let product_docs = match self
            .storage
            .list_documents_by_id_prefix(crate::core::product_help::DOCUMENT_ID_PREFIX, 512)
            .await
        {
            Ok(docs) if !docs.is_empty() => docs,
            Ok(_) => return,
            Err(error) => {
                tracing::debug!(
                    target: "security.inbound",
                    error = %error,
                    "product-help route enrichment could not load indexed help documents"
                );
                return;
            }
        };
        let hits = match crate::core::document_search::search_document_models(
            &self.storage,
            self.embedding_client.as_deref(),
            &query,
            4,
            product_docs,
        )
        .await
        {
            Ok(hits) => hits,
            Err(error) => {
                tracing::debug!(
                    target: "security.inbound",
                    error = %error,
                    "product-help route enrichment search failed"
                );
                return;
            }
        };
        let mut seen = HashSet::new();
        let doc_ids = hits
            .into_iter()
            .filter_map(|hit| {
                (hit.document_id
                    .starts_with(crate::core::product_help::DOCUMENT_ID_PREFIX)
                    && seen.insert(hit.document_id.clone()))
                .then_some(hit.document_id)
            })
            .take(4)
            .collect::<Vec<_>>();
        if doc_ids.is_empty() {
            return;
        }
        routing.grounding_doc_ids = doc_ids.clone();
    }

    async fn run_direct_local_conversation_response(
        &self,
        routing: &crate::security::intent_classifier::InboundRoutingSignal,
        message: &str,
        conversation_key: &str,
    ) -> Option<String> {
        let mode = self.classify_direct_local_answer_mode(routing).await?;
        let history = self.conversation_history.read().await;
        let messages = history.get(conversation_key)?;
        let current_message = message.trim();
        let previous_for_role = |role: &str| {
            messages.iter().rev().find_map(|item| {
                if !item.role.eq_ignore_ascii_case(role) {
                    return None;
                }
                let content = crate::security::redact_secret_input(&item.content).text;
                let content = content.trim();
                if content.is_empty() || content == current_message {
                    return None;
                }
                Some(safe_truncate(content, 700))
            })
        };
        match mode {
            DirectLocalAnswerMode::PreviousUserMessage => {
                let previous = previous_for_role("user")?;
                let label = if previous.trim_end().ends_with('?') {
                    "question"
                } else {
                    "message"
                };
                Some(format!("Your previous {label} was: \"{previous}\""))
            }
            DirectLocalAnswerMode::PreviousAssistantMessage => {
                let previous = previous_for_role("assistant")?;
                Some(format!("My previous response was: \"{previous}\""))
            }
            DirectLocalAnswerMode::RecentConversationSummary => {
                let recent = messages
                    .iter()
                    .rev()
                    .filter_map(|item| {
                        let role = if item.role.eq_ignore_ascii_case("user") {
                            "You"
                        } else if item.role.eq_ignore_ascii_case("assistant") {
                            crate::branding::PRODUCT_NAME
                        } else {
                            return None;
                        };
                        let content = crate::security::redact_secret_input(&item.content).text;
                        let content = content.trim();
                        if content.is_empty() || content == current_message {
                            return None;
                        }
                        Some(format!("{}: {}", role, safe_truncate(content, 260)))
                    })
                    .take(6)
                    .collect::<Vec<_>>();
                if recent.is_empty() {
                    return None;
                }
                let summary = recent.into_iter().rev().collect::<Vec<_>>().join("\n");
                Some(format!("Recent conversation:\n{summary}"))
            }
        }
    }

    async fn run_direct_memory_response(
        &self,
        routing: Option<&crate::security::intent_classifier::InboundRoutingSignal>,
        conversation_key: &str,
        project_id: Option<&str>,
    ) -> Option<String> {
        let signal = routing?;
        if !signal.saved_user_facts_expected {
            return None;
        }
        let items = match self
            .storage
            .list_active_experience_items(
                SAVED_USER_FACT_PROMPT_KINDS,
                project_id,
                Some(conversation_key),
                DIRECT_MEMORY_MAX_ITEMS,
            )
            .await
        {
            Ok(items) => items,
            Err(error) => {
                tracing::debug!("Direct memory lookup failed: {}", error);
                return None;
            }
        };
        select_direct_memory_answer(
            &items,
            signal.profile_lookup_kind.as_deref(),
            project_id,
            Some(conversation_key),
            chrono::Utc::now(),
        )
    }

    async fn run_direct_conversation_response(
        &self,
        channel: &str,
        message: &str,
        conversation_key: &str,
        project_id: Option<&str>,
        skip_context_lookup: bool,
    ) -> std::result::Result<DirectConversationResponse, crate::core::UserFacingOutcome> {
        let recent_messages = self
            .direct_conversation_recent_messages(conversation_key)
            .await;
        let saved_user_facts_context = if skip_context_lookup {
            None
        } else {
            self.build_saved_user_facts_context(project_id, Some(conversation_key), message)
                .await
        };
        let recent_artifacts = if skip_context_lookup {
            Vec::new()
        } else {
            Self::conversation_artifacts_for_prompt(
                &self.load_recent_artifact_contexts(conversation_key).await,
                DIRECT_CONVERSATION_RECENT_ARTIFACTS,
            )
        };
        let user_prompt = direct_conversation_user_prompt(
            message,
            conversation_key,
            &recent_messages,
            &recent_artifacts,
            saved_user_facts_context.as_deref(),
        );
        let response = self
            .supervised_internal_chat_detailed(
                channel,
                "direct_conversation",
                DIRECT_CONVERSATION_VERSION,
                &ModelRole::Fast,
                self.llm_candidates_for_role(&ModelRole::Fast),
                &direct_conversation_system_prompt(),
                &user_prompt,
                &[],
                &[],
                DIRECT_CONVERSATION_TIMEOUT_MS,
                DIRECT_CONVERSATION_MAX_CANDIDATES,
            )
            .await?;

        let answer = response.content.trim();
        if answer.is_empty() {
            tracing::warn!(
                "Direct conversation responder returned an empty direct answer; falling back to agent loop"
            );
            return Ok(DirectConversationResponse::Declined {
                kind: Some(DirectConversationDeclineKind::Unknown),
                rationale: Some("empty model output".to_string()),
            });
        }
        let Some(parsed) = extract_direct_conversation_json_object(answer)
            .and_then(|value| serde_json::from_value::<DirectConversationModelOutput>(value).ok())
        else {
            tracing::warn!(
                "Direct conversation responder returned non-JSON output; falling back to agent loop"
            );
            return Ok(DirectConversationResponse::Declined {
                kind: Some(DirectConversationDeclineKind::Unknown),
                rationale: Some("non-JSON model output".to_string()),
            });
        };
        if !parsed.can_answer_directly {
            if let Some(rationale) = parsed.rationale.as_deref() {
                tracing::info!(
                    rationale = %safe_truncate(rationale, 240),
                    "Direct conversation responder requested agent loop fallback"
                );
            }
            return Ok(DirectConversationResponse::Declined {
                kind: parsed.decline_kind,
                rationale: parsed.rationale,
            });
        }
        let parsed_answer = parsed.answer.trim();
        if !parsed_answer.is_empty() {
            return Ok(DirectConversationResponse::Answer(
                parsed_answer.to_string(),
            ));
        }
        tracing::warn!(
            "Direct conversation responder returned an empty structured answer; falling back to agent loop"
        );
        Ok(DirectConversationResponse::Declined {
            kind: Some(DirectConversationDeclineKind::Unknown),
            rationale: Some("empty structured answer".to_string()),
        })
    }

    async fn respond_if_abuse_tracker_suppressed(
        &self,
        channel: &str,
        stored_user_message: &str,
        conversation_key: &str,
        is_new_conversation: bool,
        project_id: Option<&str>,
        user_message_already_recorded: bool,
    ) -> Result<Option<ProcessedMessage>> {
        let abuse_source = crate::security::abuse_tracker::SourceKey {
            channel_id: channel.to_string(),
            user_identity: None,
        };
        let abuse_tracker = crate::security::abuse_tracker::AbuseTracker::new(
            self.storage.db(),
            self.config.security.abuse_tracker.clone(),
        );
        match abuse_tracker.current_status(&abuse_source).await {
            Ok(status) if status.should_suppress_responses() => {
                let reply = match status {
                    crate::security::abuse_tracker::TrackerStatus::PendingApproval => {
                        "This channel is paused pending an operator review. Please wait - your administrator will decide whether to resume or pause further messages."
                    }
                    crate::security::abuse_tracker::TrackerStatus::Paused => {
                        "This channel has been paused by an operator. Please contact your administrator."
                    }
                    crate::security::abuse_tracker::TrackerStatus::Normal => unreachable!(),
                };
                let processed = self
                    .persist_immediate_exchange(
                        stored_user_message,
                        reply,
                        ImmediateExchangeContext {
                            channel,
                            conversation_key,
                            is_new_conversation,
                            project_id,
                            model_used: "security_guard",
                            user_message_already_recorded,
                            memory_capture_allowed: false,
                            memory_capture_source: None,
                        },
                    )
                    .await?;
                Ok(Some(processed))
            }
            Err(error) => {
                tracing::warn!(
                    target: "security.abuse",
                    channel = %channel,
                    error = %error,
                    "abuse_tracker status lookup failed; continuing with inbound guard"
                );
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Process an incoming message and generate a response
    pub async fn process_message_with_meta(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<ProcessedMessage> {
        self.process_message_with_meta_and_hints(
            message,
            channel,
            conversation_id,
            project_id,
            RequestExecutionHints::default(),
        )
        .await
    }

    pub async fn process_message_with_meta_and_hints(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        request_hints: RequestExecutionHints,
    ) -> Result<ProcessedMessage> {
        self.process_turn_request(
            message,
            channel,
            conversation_id,
            project_id,
            request_hints,
            false,
            false,
            None,
        )
        .await
    }

    /// Process an incoming message and return only response text.
    pub async fn process_message(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<String> {
        let processed = self
            .process_message_with_meta(message, channel, conversation_id, project_id)
            .await?;
        Ok(Self::render_plain_channel_response(processed))
    }

    /// Process a message with per-request trace + streaming tokens/tools.
    pub async fn process_message_stream_with_meta(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        trace_override: Arc<RwLock<ExecutionTrace>>,
        token_tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<ProcessedMessage> {
        self.process_message_stream_with_meta_and_hints(
            message,
            channel,
            conversation_id,
            project_id,
            trace_override,
            token_tx,
            RequestExecutionHints::default(),
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Public streaming API preserves existing call sites"
    )]
    pub async fn process_message_stream_with_meta_and_hints(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        _trace_override: Arc<RwLock<ExecutionTrace>>,
        token_tx: tokio::sync::mpsc::Sender<StreamEvent>,
        request_hints: RequestExecutionHints,
    ) -> Result<ProcessedMessage> {
        let fallback_tx = token_tx.clone();
        match self
            .process_turn_request(
                message,
                channel,
                conversation_id,
                project_id,
                request_hints,
                false,
                false,
                Some(token_tx.clone()),
            )
            .await
        {
            Ok(processed) => {
                queue_stream_event(
                    &fallback_tx,
                    StreamEvent::ToolProgress {
                        name: "turn".to_string(),
                        content: processed.response.clone(),
                        payload: Some(serde_json::json!({
                            "kind": "turn_completed",
                            "run_status": processed.run_status.clone(),
                        })),
                    },
                );
                Ok(processed)
            }
            Err(error) => Err(error),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Public streaming API preserves existing call sites"
    )]
    pub async fn process_message_stream_resume_with_meta_and_hints(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        _trace_override: Arc<RwLock<ExecutionTrace>>,
        token_tx: tokio::sync::mpsc::Sender<StreamEvent>,
        request_hints: RequestExecutionHints,
    ) -> Result<ProcessedMessage> {
        let fallback_tx = token_tx.clone();
        match self
            .process_turn_request(
                message,
                channel,
                conversation_id,
                project_id,
                request_hints,
                true,
                true,
                Some(token_tx.clone()),
            )
            .await
        {
            Ok(processed) => {
                queue_stream_event(
                    &fallback_tx,
                    StreamEvent::ToolProgress {
                        name: "turn".to_string(),
                        content: processed.response.clone(),
                        payload: Some(serde_json::json!({
                            "kind": "turn_completed",
                            "run_status": processed.run_status.clone(),
                        })),
                    },
                );
                Ok(processed)
            }
            Err(error) => Err(error),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Shared turn request envelope spans chat, streaming resume, and task follow-up entrypoints"
    )]
    pub(super) async fn process_turn_request(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        _project_id: Option<&str>,
        mut request_hints: RequestExecutionHints,
        user_message_already_recorded: bool,
        skip_inbound_security_precheck: bool,
        stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<ProcessedMessage> {
        let project_id: Option<&str> = None;
        let _active_request = self.track_active_message_request();
        *self.last_activity.write().await = Some(chrono::Utc::now());

        let secret_redaction = crate::security::redact_secret_input(message);
        if secret_redaction.had_secret() {
            tracing::warn!(
                "Security: redacted likely secret input from channel={} ({} match(es))",
                channel,
                secret_redaction.redactions.len()
            );
        }
        let message_storage = redact_chat_message_for_storage(&secret_redaction.text);
        let contact_info_memory_candidate =
            !secret_redaction.had_secret() && has_contact_info_for_memory_capture(&message_storage);
        let early_safe_message = message_storage.clone();
        let (resolved_conversation_id, is_new_conversation) = self
            .resolve_conversation_id(channel, conversation_id, project_id, &early_safe_message)
            .await?;
        let conversation_key = resolved_conversation_id.clone();

        let mut memory_capture_allowed = false;
        let mut memory_capture_allowed_from_semantic_probe = false;
        let mut inbound_routing_trusted = false;
        let mut inbound_router_unavailable = false;
        let mut inbound_direct_response: Option<String> = None;
        let turn_started_at = chrono::Utc::now();
        let usage_before_turn = self.turn_pipeline_usage_snapshot().await;
        if !skip_inbound_security_precheck {
            if secret_redaction.had_secret() {
                if let Some(processed) = self
                    .respond_if_abuse_tracker_suppressed(
                        channel,
                        &message_storage,
                        &conversation_key,
                        is_new_conversation,
                        project_id,
                        user_message_already_recorded,
                    )
                    .await?
                {
                    return Ok(processed);
                }
                let pending_chat_credential_prompt =
                    self.pending_chat_credential_prompt(&conversation_key).await;
                let safe_reply = if pending_chat_credential_prompt.is_some() {
                    "I redacted the secret from this chat message. Submit credentials through the secure credential form instead; I can't use, test, or save secrets pasted into normal chat."
                } else {
                    "I redacted the secret from this chat message. I can't use, test, or save secrets pasted into normal chat. Add credentials through the secure Settings or integration credential flow, then tell me what you want to do."
                };
                let processed = self
                    .persist_immediate_exchange(
                        &message_storage,
                        safe_reply,
                        ImmediateExchangeContext {
                            channel,
                            conversation_key: &conversation_key,
                            is_new_conversation,
                            project_id,
                            model_used: "secret_input_guard",
                            user_message_already_recorded,
                            memory_capture_allowed: false,
                            memory_capture_source: None,
                        },
                    )
                    .await?;
                return Ok(processed);
            }

            if let Some(tx) = stream_tx.as_ref() {
                queue_stream_event(
                    tx,
                    StreamEvent::Thinking("Checking request safety...".to_string()),
                );
            }
            match self
                .run_inbound_security_precheck(
                    &message_storage,
                    &message_storage,
                    channel,
                    &conversation_key,
                    is_new_conversation,
                    project_id,
                    user_message_already_recorded,
                    stream_tx.as_ref(),
                )
                .await?
            {
                InboundSecurityPrecheck::Respond(processed) => return Ok(processed),
                InboundSecurityPrecheck::Continue {
                    memory_capture_allowed: should_capture,
                    routing,
                    routing_trusted,
                    direct_response,
                } => {
                    memory_capture_allowed = should_capture;
                    inbound_routing_trusted = routing_trusted;
                    inbound_direct_response = direct_response;
                    request_hints.routing_trusted = routing_trusted;
                    inbound_router_unavailable = !routing_trusted && routing.is_none();
                    if let Some(routing) = routing {
                        request_hints.routing = Some(routing);
                    }
                }
            }
        }

        if secret_redaction.had_secret() {
            memory_capture_allowed = false;
            let pending_chat_credential_prompt =
                self.pending_chat_credential_prompt(&conversation_key).await;
            let secure_prompt_pending = pending_chat_credential_prompt.is_some();
            let kind = match secret_redaction
                .primary_kind()
                .unwrap_or(crate::security::SecretInputType::ApiKeyOrToken)
            {
                crate::security::SecretInputType::PrivateKeyMaterial => "private_key_material",
                crate::security::SecretInputType::ApiKeyOrToken => "api_key_or_token",
                crate::security::SecretInputType::PaymentCredential => "payment_credential",
            };
            request_hints.secret_offered = Some(SecretOfferedHint {
                kind: kind.to_string(),
                redactions: secret_redaction.redactions.clone(),
                secure_prompt_pending,
            });
        }
        if contact_info_memory_candidate {
            memory_capture_allowed = true;
        }

        let direct_candidate_state = DirectConversationRuntimeState {
            routing_trusted: inbound_routing_trusted,
            has_attachments: !request_hints.attachments.is_empty(),
            has_secret_offered: request_hints.secret_offered.is_some(),
            has_pending_actions: false,
            has_pending_credential_prompt: false,
            user_message_already_recorded,
            skip_inbound_security_precheck,
            supported_surface: matches!(
                request_hints.execution_surface,
                ActionExecutionSurface::Chat | ActionExecutionSurface::Api
            ),
        };
        let new_empty_conversation = is_new_conversation && !user_message_already_recorded;
        let direct_candidate_path = turn_execution_path_from_routing(
            request_hints.routing.as_ref(),
            direct_candidate_state,
        );
        if !memory_capture_allowed
            && !secret_redaction.had_secret()
            && !routing_is_transient_read_only_lookup(request_hints.routing.as_ref())
            && should_enqueue_semantic_user_memory_capture(
                message_storage.as_str(),
                direct_candidate_state,
                direct_candidate_path,
            )
        {
            memory_capture_allowed = true;
            memory_capture_allowed_from_semantic_probe = true;
        }
        if !memory_capture_allowed
            && inbound_router_unavailable
            && !secret_redaction.had_secret()
            && direct_candidate_path == TurnExecutionPath::DirectReply
        {
            memory_capture_allowed = true;
            memory_capture_allowed_from_semantic_probe = true;
        }
        let raw_memory_capture_source = if memory_capture_allowed && !secret_redaction.had_secret()
        {
            Some(message_storage.as_str())
        } else {
            None
        };
        let direct_reply_read_only_yield_check_needed = request_hints
            .routing
            .as_ref()
            .map(super::action_selection::routing_signal_has_read_only_retrieval_need)
            .unwrap_or(false)
            || (request_hints.routing.is_some() && !direct_candidate_state.routing_trusted);
        if direct_candidate_path == TurnExecutionPath::DirectReply
            && direct_reply_read_only_yield_check_needed
            && self
                .direct_reply_should_yield_to_read_only_action(
                    message_storage.as_str(),
                    &request_hints,
                )
                .await
        {
            request_hints.force_agent_loop = true;
        }

        if !request_hints.force_agent_loop && direct_candidate_path == TurnExecutionPath::DirectReply
        {
            let mut direct_conversation_declined = false;
            let mut direct_conversation_decline_kind = None;
            let mut direct_conversation_decline_rationale: Option<String> = None;
            let pending_actions = if new_empty_conversation {
                Vec::new()
            } else {
                self.pending_conversation_actions(&conversation_key).await
            };
            let pending_credential_prompt = if new_empty_conversation {
                false
            } else {
                self.pending_chat_credential_prompt(&conversation_key)
                    .await
                    .is_some()
            };
            let direct_state = DirectConversationRuntimeState {
                has_pending_actions: !pending_actions.is_empty(),
                has_pending_credential_prompt: pending_credential_prompt,
                ..direct_candidate_state
            };
            let direct_reply_available =
                turn_execution_path_from_routing(request_hints.routing.as_ref(), direct_state)
                    == TurnExecutionPath::DirectReply;
            if direct_reply_available {
                if let Some(response) = self
                    .run_direct_memory_response(
                        request_hints.routing.as_ref(),
                        &conversation_key,
                        project_id,
                    )
                    .await
                {
                    let usage_delta = self
                        .turn_pipeline_usage_snapshot()
                        .await
                        .delta_since(usage_before_turn);
                    return self
                        .persist_turn_pipeline_exchange(
                            message_storage.as_str(),
                            &response,
                            ImmediateExchangeContext {
                                channel,
                                conversation_key: &conversation_key,
                                is_new_conversation,
                                project_id,
                                model_used: DIRECT_MEMORY_MODEL_USED,
                                user_message_already_recorded,
                                memory_capture_allowed,
                                memory_capture_source: raw_memory_capture_source,
                            },
                            crate::core::ExecutionRunStatus::Completed.as_str(),
                            Vec::new(),
                            Vec::new(),
                            None,
                            turn_started_at,
                            usage_delta,
                        )
                        .await;
                }
                let direct_local_response = match request_hints.routing.as_ref() {
                    Some(routing) => {
                        self.run_direct_local_conversation_response(
                            routing,
                            message_storage.as_str(),
                            &conversation_key,
                        )
                        .await
                    }
                    None => None,
                };
                if let Some(response) = direct_local_response {
                    let usage_delta = self
                        .turn_pipeline_usage_snapshot()
                        .await
                        .delta_since(usage_before_turn);
                    return self
                        .persist_turn_pipeline_exchange(
                            message_storage.as_str(),
                            &response,
                            ImmediateExchangeContext {
                                channel,
                                conversation_key: &conversation_key,
                                is_new_conversation,
                                project_id,
                                model_used: DIRECT_CONTEXT_MODEL_USED,
                                user_message_already_recorded,
                                memory_capture_allowed,
                                memory_capture_source: raw_memory_capture_source,
                            },
                            crate::core::ExecutionRunStatus::Completed.as_str(),
                            Vec::new(),
                            Vec::new(),
                            None,
                            turn_started_at,
                            usage_delta,
                        )
                        .await;
                }
                if let Some(response) = inbound_direct_response
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    let usage_delta = self
                        .turn_pipeline_usage_snapshot()
                        .await
                        .delta_since(usage_before_turn);
                    return self
                        .persist_turn_pipeline_exchange(
                            message_storage.as_str(),
                            response,
                            ImmediateExchangeContext {
                                channel,
                                conversation_key: &conversation_key,
                                is_new_conversation,
                                project_id,
                                model_used: "inbound_classifier_direct",
                                user_message_already_recorded,
                                memory_capture_allowed,
                                memory_capture_source: raw_memory_capture_source,
                            },
                            crate::core::ExecutionRunStatus::Completed.as_str(),
                            Vec::new(),
                            Vec::new(),
                            None,
                            turn_started_at,
                            usage_delta,
                        )
                        .await;
                }
                if let Some(tx) = stream_tx.as_ref() {
                    queue_stream_event(
                        tx,
                        StreamEvent::Thinking("Answering directly...".to_string()),
                    );
                }
                match self
                    .run_direct_conversation_response(
                        channel,
                        message_storage.as_str(),
                        &conversation_key,
                        project_id,
                        new_empty_conversation,
                    )
                    .await
                {
                    Ok(DirectConversationResponse::Answer(response)) => {
                        let usage_delta = self
                            .turn_pipeline_usage_snapshot()
                            .await
                            .delta_since(usage_before_turn);
                        return self
                            .persist_turn_pipeline_exchange(
                                message_storage.as_str(),
                                &response,
                                ImmediateExchangeContext {
                                    channel,
                                    conversation_key: &conversation_key,
                                    is_new_conversation,
                                    project_id,
                                    model_used: DIRECT_CONVERSATION_MODEL_USED,
                                    user_message_already_recorded,
                                    memory_capture_allowed,
                                    memory_capture_source: raw_memory_capture_source,
                                },
                                crate::core::ExecutionRunStatus::Completed.as_str(),
                                Vec::new(),
                                Vec::new(),
                                None,
                                turn_started_at,
                                usage_delta,
                            )
                            .await;
                    }
                    Ok(DirectConversationResponse::Declined { kind, rationale }) => {
                        direct_conversation_decline_kind = kind;
                        direct_conversation_decline_rationale = rationale.clone();
                        if let Some(rationale) = rationale.as_deref() {
                            tracing::info!(
                                decline_kind = ?kind,
                                rationale = %safe_truncate(rationale, 240),
                                "Direct conversation path declined by semantic responder; falling back to agent loop"
                            );
                        } else {
                            tracing::info!(
                                decline_kind = ?kind,
                                "Direct conversation path declined by semantic responder; falling back to agent loop"
                            );
                        }
                        direct_conversation_declined = true;
                    }
                    Err(outcome) => {
                        direct_conversation_declined = true;
                        tracing::warn!(
                            reason = %safe_truncate(&outcome.message, 240),
                            "Direct conversation path failed; falling back to agent loop"
                        );
                    }
                }
            }
            if direct_conversation_declined {
                request_hints.force_agent_loop = true;
                if neutralize_direct_reply_routing_after_direct_decline(
                    request_hints.routing.as_mut(),
                    direct_state,
                    direct_conversation_decline_kind,
                    message_storage.as_str(),
                    direct_conversation_decline_rationale.as_deref(),
                ) {
                    tracing::info!(
                        "Direct conversation decline invalidated direct-reply routing; enabling execution planning"
                    );
                }
                if memory_capture_allowed_from_semantic_probe
                    && routing_is_transient_read_only_lookup(request_hints.routing.as_ref())
                {
                    memory_capture_allowed = false;
                }
            }
        }

        if is_new_conversation && !conversation_key.is_empty() {
            self.ensure_conversation_row_for_turn(
                &conversation_key,
                channel,
                project_id,
                message_storage.as_str(),
                None,
            )
            .await?;
        }

        match self
            .run_agent_turn_loop_for_chat(
                channel,
                message_storage.as_str(),
                Some(&conversation_key),
                project_id,
                &request_hints,
                stream_tx.clone(),
            )
            .await
        {
            Ok(processed) => {
                let usage_delta = self
                    .turn_pipeline_usage_snapshot()
                    .await
                    .delta_since(usage_before_turn);
                let attachment_memory_source = attachment_only_visual_memory_capture_source(
                    message_storage.as_str(),
                    &processed.response,
                    &request_hints,
                );
                let durable_work_memory_source = memory_capture_source_with_completed_work_context(
                    message_storage.as_str(),
                    &processed.response,
                    &processed.turn_records,
                    processed.turn_plan.as_ref(),
                );
                let run_allows_memory_capture = processed
                    .run_status
                    .as_deref()
                    .map(|status| matches!(status, "completed" | "completed_degraded"))
                    .unwrap_or(true);
                let memory_capture_source = if run_allows_memory_capture {
                    attachment_memory_source
                        .as_deref()
                        .or(durable_work_memory_source.as_deref())
                        .or(raw_memory_capture_source)
                } else {
                    None
                };
                self.persist_turn_pipeline_exchange(
                    message_storage.as_str(),
                    &processed.response,
                    ImmediateExchangeContext {
                        channel,
                        conversation_key: &conversation_key,
                        is_new_conversation,
                        project_id,
                        model_used: "agent_turn_loop",
                        user_message_already_recorded,
                        memory_capture_allowed: run_allows_memory_capture
                            && (memory_capture_allowed || attachment_memory_source.is_some()),
                        memory_capture_source,
                    },
                    processed.run_status.as_deref().unwrap_or("completed"),
                    processed.trace_steps.clone(),
                    processed.turn_records.clone(),
                    processed.turn_plan.clone(),
                    turn_started_at,
                    usage_delta,
                )
                .await
            }
            Err(error) => {
                if error.to_string() == "Conversation not found" {
                    return Err(error);
                }
                tracing::warn!("Agent turn loop failed on channel '{}': {}", channel, error);
                let response = format!(
                    "The agent turn loop hit a framework-level failure before execution could complete, so I did not run any action. Please retry after checking the server logs. Error: {}",
                    error
                );
                let usage_delta = self
                    .turn_pipeline_usage_snapshot()
                    .await
                    .delta_since(usage_before_turn);
                self.persist_turn_pipeline_exchange(
                    message_storage.as_str(),
                    &response,
                    ImmediateExchangeContext {
                        channel,
                        conversation_key: &conversation_key,
                        is_new_conversation,
                        project_id,
                        model_used: "agent_turn_loop_failed",
                        user_message_already_recorded,
                        memory_capture_allowed: false,
                        memory_capture_source: None,
                    },
                    crate::core::ExecutionRunStatus::PlatformFailed.as_str(),
                    Vec::new(),
                    Vec::new(),
                    None,
                    turn_started_at,
                    usage_delta,
                )
                .await
            }
        }
    }

    async fn turn_pipeline_usage_snapshot(&self) -> TurnPipelineUsageSnapshot {
        let trace = self.last_trace.read().await;
        TurnPipelineUsageSnapshot {
            input_tokens: trace.input_tokens,
            output_tokens: trace.output_tokens,
            total_tokens: trace.total_tokens,
            cost_usd: trace.cost_usd,
        }
    }

    async fn ensure_conversation_row_for_turn(
        &self,
        conversation_id: &str,
        channel: &str,
        project_id: Option<&str>,
        message_preview: &str,
        conversation_title: Option<&str>,
    ) -> Result<()> {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return Ok(());
        }

        let now = chrono::Utc::now().to_rfc3339();
        let title = conversation_title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| safe_truncate(value, 80))
            .unwrap_or_else(|| safe_truncate(message_preview, 50));
        let conv = crate::storage::entities::conversation::Model {
            id: conversation_id.to_string(),
            title,
            channel: channel.to_string(),
            project_id: project_id.map(str::to_string),
            created_at: now.clone(),
            updated_at: now,
            message_count: 0,
            archived: false,
            starred: false,
        };
        self.storage.create_conversation_if_absent(&conv).await?;
        Ok(())
    }

    async fn remember_completed_trace_snapshot(
        &self,
        trace_snapshot: ExecutionTrace,
        update_last_trace: bool,
    ) {
        if trace_snapshot.id.trim().is_empty() {
            return;
        }

        {
            let mut history = self.trace_history.write().await;
            history.retain(|item| item.id != trace_snapshot.id);
            history.insert(0, trace_snapshot.clone());
            if history.len() > 100 {
                history.truncate(100);
            }
        }

        if update_last_trace {
            *self.last_trace.write().await = trace_snapshot;
        }
    }

    fn spawn_deferred_exchange_persistence(&self, job: DeferredExchangePersistence) {
        let agent = self.clone();
        let trace_id = job.trace_snapshot.id.clone();
        let pending = DEFERRED_CHAT_PERSISTENCE_PENDING
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        if pending >= DEFERRED_CHAT_PERSISTENCE_WARN_PENDING {
            tracing::warn!(
                pending,
                "Deferred chat persistence backlog is high; responses are still unblocked"
            );
        }

        tokio::spawn(async move {
            let semaphore = DEFERRED_CHAT_PERSISTENCE_SEMAPHORE.clone();
            let _permit = match semaphore.acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => {
                    DEFERRED_CHAT_PERSISTENCE_PENDING
                        .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    tracing::error!(
                        trace_id,
                        "Deferred chat persistence limiter closed before persistence could run"
                    );
                    return;
                }
            };

            let mut last_error: Option<String> = None;
            let mut persisted = false;
            for attempt in 1..=DEFERRED_CHAT_PERSISTENCE_ATTEMPTS {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(DEFERRED_CHAT_PERSISTENCE_ATTEMPT_TIMEOUT_SECS),
                    agent.persist_deferred_exchange_once(job.clone()),
                )
                .await;

                match result {
                    Ok(Ok(())) => {
                        persisted = true;
                        break;
                    }
                    Ok(Err(error)) => {
                        last_error = Some(error.to_string());
                    }
                    Err(_) => {
                        last_error = Some(format!(
                            "attempt timed out after {} seconds",
                            DEFERRED_CHAT_PERSISTENCE_ATTEMPT_TIMEOUT_SECS
                        ));
                    }
                }

                if attempt < DEFERRED_CHAT_PERSISTENCE_ATTEMPTS {
                    let delay_secs = match attempt {
                        1 => 1,
                        2 => 3,
                        _ => 8,
                    };
                    tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
                }
            }

            if !persisted {
                tracing::error!(
                    trace_id,
                    error = last_error.as_deref().unwrap_or("unknown"),
                    "Deferred chat persistence failed after retries"
                );
            }
            DEFERRED_CHAT_PERSISTENCE_PENDING.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        });
    }

    async fn persist_deferred_exchange_once(&self, job: DeferredExchangePersistence) -> Result<()> {
        let trace_id = job.trace_snapshot.id.clone();
        self.persist_completed_trace_snapshot_durable(&job.trace_snapshot)
            .await?;

        if job.kind == DeferredExchangePersistenceKind::Immediate {
            let started_payload = serde_json::json!({
                "flow_kind": "immediate",
                "message_chars": job.message.chars().count(),
                "resumed": job.user_message_already_recorded,
            });
            self.log_operational_event(operational::OperationalEvent {
                event_type: "agent_request",
                channel: &job.channel,
                success: true,
                outcome: "started",
                trace_id: Some(&trace_id),
                conversation_id: Some(&job.conversation_key),
                tool_name: None,
                latency_ms: Some(0),
                arguments: None,
                payload: Some(&started_payload),
                strategy_version: None,
                policy_version: None,
                prompt_version: None,
                specialist_prompt_version: None,
                model_slot: Some(&job.model_used),
            })
            .await;

            let completed_payload = serde_json::json!({
                "response_chars": job.response.chars().count(),
                "tool_calls": 0,
                "degradation_notes": 0,
                "status": "completed",
            });
            self.log_operational_event(operational::OperationalEvent {
                event_type: "response_complete",
                channel: &job.channel,
                success: true,
                outcome: "completed",
                trace_id: Some(&trace_id),
                conversation_id: Some(&job.conversation_key),
                tool_name: None,
                latency_ms: Some(0),
                arguments: None,
                payload: Some(&completed_payload),
                strategy_version: None,
                policy_version: None,
                prompt_version: None,
                specialist_prompt_version: None,
                model_slot: Some(&job.model_used),
            })
            .await;
        }

        if !job.conversation_key.is_empty() {
            if job.is_new_conversation {
                self.ensure_conversation_row_for_turn(
                    &job.conversation_key,
                    &job.channel,
                    job.project_id.as_deref(),
                    &job.message,
                    job.conversation_title.as_deref(),
                )
                .await?;
            }
            if !job.user_message_already_recorded {
                let user_msg = crate::storage::entities::message::Model {
                    id: job.user_message_id.clone(),
                    conversation_id: job.conversation_key.clone(),
                    role: "user".to_string(),
                    content: job.message.clone(),
                    timestamp: job.user_timestamp.clone(),
                    model_used: None,
                    trace_id: Some(trace_id.clone()),
                };
                self.encrypted_storage
                    .insert_message_encrypted_if_absent(&user_msg)
                    .await?;
                if job.memory_capture_allowed {
                    let memory_source = job
                        .memory_capture_source
                        .as_deref()
                        .unwrap_or(job.message.as_str());
                    let queued_memory_capture = self
                        .mark_user_memory_capture_candidate(
                            memory_source,
                            &job.channel,
                            Some(&job.conversation_key),
                            job.project_id.as_deref(),
                            Some(&user_msg.id),
                        )
                        .await;
                    if queued_memory_capture {
                        self.kick_deferred_user_memory_capture_processing();
                    }
                }
            }

            let asst_msg = crate::storage::entities::message::Model {
                id: job.assistant_message_id.clone(),
                conversation_id: job.conversation_key.clone(),
                role: "assistant".to_string(),
                content: job.response.clone(),
                timestamp: job.assistant_timestamp.clone(),
                model_used: Some(job.model_used.clone()),
                trace_id: Some(trace_id.clone()),
            };
            self.encrypted_storage
                .insert_message_encrypted_if_absent(&asst_msg)
                .await?;

            if job.is_new_conversation {
                if let Some(title) = job.conversation_title.as_deref() {
                    self.storage
                        .update_conversation(&job.conversation_key, Some(title), None, None)
                        .await?;
                }
            }

            self.sync_background_session_after_response(
                &job.conversation_key,
                &job.message,
                &job.response,
            )
            .await;
            self.sync_pending_resilience_followup(
                &job.conversation_key,
                &job.message,
                &job.channel,
                job.project_id.as_deref(),
                &job.user_outcome,
            )
            .await;
        }

        self.record_completed_interaction_for_self_tune().await;
        Ok(())
    }

    async fn persist_turn_pipeline_exchange(
        &self,
        message: &str,
        response: &str,
        context: ImmediateExchangeContext<'_>,
        run_status: &str,
        trace_steps: Vec<ExecutionStep>,
        turn_records: Vec<AgentTurnRecord>,
        turn_plan: Option<ExecutionPlan>,
        started_at: chrono::DateTime<chrono::Utc>,
        usage_delta: TurnPipelineUsageSnapshot,
    ) -> Result<ProcessedMessage> {
        let trace_id = uuid::Uuid::new_v4().to_string();
        let run_id = uuid::Uuid::new_v4().to_string();
        let trace_time = chrono::Utc::now();
        let first_content_ms = (trace_time - started_at).num_milliseconds().max(1) as u64;
        let filtered_response = self.security.filter_output(response);
        if !filtered_response.redactions.is_empty() {
            tracing::warn!(
                "Security: redacted sensitive data from turn output before persistence ({} rule match(es))",
                filtered_response.redactions.len()
            );
        }
        let safe_response = filtered_response.text;
        let is_direct_memory = context.model_used == DIRECT_MEMORY_MODEL_USED;
        let is_direct_conversation = context.model_used == DIRECT_CONVERSATION_MODEL_USED;
        let flow_label = if is_direct_memory {
            "Direct memory"
        } else if is_direct_conversation {
            "Direct conversation"
        } else {
            "Agent turn loop"
        };
        let complexity = if is_direct_memory {
            "direct_memory"
        } else if is_direct_conversation {
            "direct_conversation"
        } else {
            "agent_turn_loop"
        };
        let first_content_source = if is_direct_memory {
            "direct_memory_first_content"
        } else if is_direct_conversation {
            "direct_conversation_first_content"
        } else {
            "agent_turn_loop_first_content"
        };
        let mut steps = Vec::with_capacity(trace_steps.len() + 3);
        steps.push(ExecutionStep {
            icon: "[turn]".to_string(),
            title: "Turn Request".to_string(),
            detail: format!(
                "{} | Channel: {} | Length: {} chars",
                flow_label,
                context.channel,
                message.chars().count()
            ),
            step_type: "info".to_string(),
            data: None,
            timestamp: started_at,
            duration_ms: Some(0),
        });
        steps.extend(trace_steps);
        steps.push(ExecutionStep {
            icon: "[model]".to_string(),
            title: "First Content".to_string(),
            detail: format!(
                "AgentArk produced the first user-visible response content after {}ms.",
                first_content_ms
            ),
            step_type: "info".to_string(),
            data: Some(
                serde_json::json!({
                    "metric": "time_to_first_token",
                    "duration_ms": first_content_ms,
                    "source": first_content_source
                })
                .to_string(),
            ),
            timestamp: trace_time,
            duration_ms: Some(first_content_ms),
        });
        steps.push(ExecutionStep {
            icon: "[reply]".to_string(),
            title: "Turn Response".to_string(),
            detail: format!(
                "Returned via {} with status '{}'.",
                context.model_used, run_status
            ),
            step_type: "success".to_string(),
            data: Some(safe_truncate(&safe_response, 8000)),
            timestamp: chrono::Utc::now(),
            duration_ms: Some(0),
        });

        let trace_ref = Arc::new(RwLock::new(ExecutionTrace {
            id: trace_id.clone(),
            message: message.to_string(),
            channel: context.channel.to_string(),
            started_at: Some(started_at),
            completed_at: Some(trace_time),
            steps,
            proof_id: None,
            response: Some(safe_response.clone()),
            model: Some(context.model_used.to_string()),
            input_tokens: usage_delta.input_tokens,
            output_tokens: usage_delta.output_tokens,
            total_tokens: usage_delta.total_tokens,
            cost_usd: usage_delta.cost_usd,
            complexity: Some(complexity.to_string()),
            plan: turn_plan.clone(),
        }));
        let trace_snapshot = trace_ref.read().await.clone();
        self.remember_completed_trace_snapshot(trace_snapshot.clone(), true)
            .await;

        {
            let mut history = self.conversation_history.write().await;
            let conversation_history = history
                .entry(context.conversation_key.to_string())
                .or_insert_with(Vec::new);
            if !context.user_message_already_recorded {
                conversation_history.push(ConversationMessage {
                    role: "user".to_string(),
                    content: message.to_string(),
                    _timestamp: chrono::Utc::now(),
                });
            }
            conversation_history.push(ConversationMessage {
                role: "assistant".to_string(),
                content: safe_response.clone(),
                _timestamp: chrono::Utc::now(),
            });
            if conversation_history.len() > 10 {
                conversation_history.drain(0..conversation_history.len() - 10);
            }
        }

        let mut conversation_title: Option<String> = None;
        if !context.conversation_key.is_empty() {
            if context.is_new_conversation {
                let title = self.generate_conversation_title(message);
                *self.last_conversation_title.write().await = Some(title.clone());
                conversation_title = Some(title);
            } else {
                *self.last_conversation_title.write().await = None;
            }
        }

        *self.last_conversation_id.write().await = Some(context.conversation_key.to_string());

        let user_outcome = self
            .build_response_heuristic_outcome(&safe_response, &[], &[], None)
            .unwrap_or_else(|| {
                self.execution_supervisor
                    .build_success_outcome(&safe_response, &[], &[])
            });
        let final_run_status = if run_status.trim().is_empty() {
            Self::execution_run_status_for_outcome(&user_outcome)
                .as_str()
                .to_string()
        } else {
            run_status.trim().to_string()
        };
        self.spawn_deferred_exchange_persistence(DeferredExchangePersistence {
            kind: DeferredExchangePersistenceKind::TurnPipeline,
            trace_snapshot,
            message: message.to_string(),
            response: safe_response.clone(),
            channel: context.channel.to_string(),
            conversation_key: context.conversation_key.to_string(),
            project_id: context.project_id.map(str::to_string),
            model_used: context.model_used.to_string(),
            user_message_already_recorded: context.user_message_already_recorded,
            memory_capture_allowed: context.memory_capture_allowed,
            memory_capture_source: context.memory_capture_source.map(str::to_string),
            user_message_id: uuid::Uuid::new_v4().to_string(),
            assistant_message_id: uuid::Uuid::new_v4().to_string(),
            user_timestamp: chrono::Utc::now().to_rfc3339(),
            assistant_timestamp: chrono::Utc::now().to_rfc3339(),
            is_new_conversation: context.is_new_conversation,
            conversation_title: conversation_title.clone(),
            user_outcome: user_outcome.clone(),
        });

        Ok(ProcessedMessage {
            response: safe_response,
            conversation_id: Some(context.conversation_key.to_string()),
            conversation_title,
            run_id: Some(run_id),
            run_status: Some(final_run_status),
            trace_id: Some(trace_id),
            input_tokens: usage_delta.input_tokens,
            output_tokens: usage_delta.output_tokens,
            total_tokens: usage_delta.total_tokens,
            choices: Vec::new(),
            degradation: Vec::new(),
            attempted_models: Vec::new(),
            user_outcome: Some(user_outcome),
            trace_steps: Vec::new(),
            turn_records,
            turn_plan,
        })
    }

    pub(crate) fn render_plain_channel_response(processed: ProcessedMessage) -> String {
        let mut response = processed.response;
        if let Some(outcome) = processed.user_outcome.as_ref() {
            let needs_prefix = match outcome.status {
                super::UserFacingOutcomeStatus::NeedsPermission => {
                    !response.to_ascii_lowercase().contains("approval")
                }
                super::UserFacingOutcomeStatus::NeedsIntegration => {
                    !response.to_ascii_lowercase().contains("integration")
                }
                super::UserFacingOutcomeStatus::NeedsCredentials => {
                    !response.to_ascii_lowercase().contains("credential")
                        && !response.to_ascii_lowercase().contains("api key")
                        && !response.to_ascii_lowercase().contains("token")
                }
                super::UserFacingOutcomeStatus::NeedsStrongerModel => {
                    !response.to_ascii_lowercase().contains("stronger model")
                }
                super::UserFacingOutcomeStatus::ServiceUnavailable => {
                    !response
                        .to_ascii_lowercase()
                        .contains("framework-level problem")
                        && !response.to_ascii_lowercase().contains("service")
                }
                _ => false,
            };

            if needs_prefix {
                let prefix = match outcome.status {
                    super::UserFacingOutcomeStatus::NeedsPermission => {
                        "Approval needed before I can continue.\n\n"
                    }
                    super::UserFacingOutcomeStatus::NeedsIntegration => {
                        "Integration setup needed before I can continue.\n\n"
                    }
                    super::UserFacingOutcomeStatus::NeedsCredentials => {
                        "Credentials or configuration are needed before I can continue.\n\n"
                    }
                    super::UserFacingOutcomeStatus::NeedsStrongerModel => {
                        "A stronger model is needed to finish this request.\n\n"
                    }
                    super::UserFacingOutcomeStatus::ServiceUnavailable => {
                        "The request stayed inside the resilience layer, but the service is currently unavailable.\n\n"
                    }
                    _ => "",
                };
                if !prefix.is_empty() {
                    response = format!("{}{}", prefix, response);
                }
            }
        }
        let should_prefix_degraded = processed
            .user_outcome
            .as_ref()
            .is_some_and(|outcome| outcome.status == super::UserFacingOutcomeStatus::Degraded)
            && processed
                .degradation
                .iter()
                .any(|note| matches!(note.kind.as_str(), "delegation" | "tool" | "tool_dispatch"));

        if should_prefix_degraded
            && !response.starts_with("Note: I completed this with partial")
            && !response.starts_with("Note: I completed this with degraded")
        {
            let prefix = if processed
                .degradation
                .iter()
                .any(|note| note.kind == "delegation")
            {
                "Note: I completed this with partial delegated coverage because one or more execution paths degraded.\n\n"
            } else {
                "Note: I completed this with degraded execution, so parts of the result may be partial.\n\n"
            };
            response = format!("{}{}", prefix, response);
        }

        response
    }

    pub(super) fn build_execution_resume_message(
        run: &crate::core::ExecutionRun,
        checkpoints: &[crate::core::ExecutionCheckpoint],
        tool_attempts: &[crate::core::ToolAttempt],
    ) -> String {
        let mut lines = vec![
            "Resume this AgentArk execution from its last completed checkpoint.".to_string(),
            "Do not restart or repeat completed work unless validation proves it is stale or missing.".to_string(),
            "Continue with the next required action/tool step, and finish only when the original goal is complete or a real blocker is reached.".to_string(),
            String::new(),
            format!("Previous run id: {}", run.id),
            format!("Previous status: {}", run.status.as_str()),
            format!("Previous stage: {}", run.current_stage),
        ];

        if let Some(original) = run
            .request_message
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            lines.push(format!("Original request: {}", original));
        }
        if let Some(summary) = run
            .result_summary
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            lines.push(format!(
                "Previous result summary: {}",
                safe_truncate(summary, 600)
            ));
        }
        if let Some(error) = run
            .last_error
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            lines.push(format!("Previous error: {}", safe_truncate(error, 600)));
        }

        lines.push(String::new());
        lines.push("Persisted checkpoints, oldest to newest:".to_string());
        if checkpoints.is_empty() {
            lines.push("- No checkpoint payloads were persisted for this run; use the run status and original request as context.".to_string());
        } else {
            let start = checkpoints.len().saturating_sub(12);
            for checkpoint in checkpoints.iter().skip(start) {
                lines.push(format!(
                    "- #{} stage={} at {} payload={}",
                    checkpoint.sequence_no,
                    checkpoint.stage,
                    checkpoint.created_at,
                    safe_truncate(&checkpoint.payload, 800)
                ));
            }
        }

        lines.push(String::new());
        lines.push("Persisted tool attempts, oldest to newest:".to_string());
        if tool_attempts.is_empty() {
            lines.push("- No persisted tool attempts were found for this run.".to_string());
        } else {
            let start = tool_attempts.len().saturating_sub(12);
            for attempt in tool_attempts.iter().skip(start) {
                let error = attempt
                    .error_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| format!(" error={}", safe_truncate(value, 300)))
                    .unwrap_or_default();
                lines.push(format!(
                    "- #{} tool={} status={} retryable={} side_effect={} args={} output={}{}",
                    attempt.sequence_no,
                    attempt.tool_name,
                    attempt.status.as_str(),
                    attempt.retryable,
                    attempt.side_effect_level,
                    safe_truncate(&attempt.arguments_json, 500),
                    safe_truncate(&attempt.output_json, 700),
                    error
                ));
            }
        }

        lines.push(String::new());
        lines.push("If the last completed step only installed dependencies, prepared files, cloned a repo, or gathered setup evidence, continue from the validation or handoff step instead of reinstalling/recloning. If a persistent object already exists, inspect/reuse it rather than creating duplicates.".to_string());
        lines.join("\n")
    }

    pub async fn resume_execution_run(
        &self,
        run_id: &str,
        caller: Option<&ActionCallerPrincipal>,
    ) -> Result<ProcessedMessage> {
        let run = self
            .storage
            .load_execution_run(run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Run not found"))?;
        let checkpoints = self.storage.load_execution_checkpoints(run_id).await?;
        let tool_attempts = self.storage.list_tool_attempts_for_run(run_id).await?;
        let resume_message =
            Self::build_execution_resume_message(&run, &checkpoints, &tool_attempts);
        let mut hints = RequestExecutionHints::default();
        hints.execution_surface = ActionExecutionSurface::Chat;
        hints.direct_user_intent = true;
        hints.caller_principal = caller.cloned();

        self.process_message_with_meta_and_hints(
            &resume_message,
            run.channel.as_deref().unwrap_or("web"),
            run.conversation_id.as_deref(),
            None,
            hints,
        )
        .await
    }

    /// Build a structured surface-context JSON for the inbound classifier.
    ///
    /// We intentionally describe the surface in capability terms — what the
    /// user can do here — not in any user-facing language. The classifier
    /// reasons from the structural shape (which capability clusters are
    /// available) rather than from any phrase. Returns `None` when the
    /// channel does not correspond to a known structural surface, so the
    /// general inbound prompt is unchanged for ordinary chat traffic.
    pub(super) async fn build_inbound_surface_context(
        &self,
        channel: &str,
    ) -> Option<serde_json::Value> {
        if channel != "arkorbit" {
            return None;
        }
        let user_id = self.identity.did().to_string();
        let orbit_count = self
            .arkorbit
            .list_orbits(&user_id)
            .await
            .map(|orbits| orbits.len())
            .unwrap_or(0);
        Some(serde_json::json!({
            "surface": "arkorbit_canvas",
            "orbit_count": orbit_count,
            "scope_policy": "an orbit must be explicitly selected or created before durable orbit file authoring",
            "orbit_file_namespace": ["index.html", "orbit.json", "mod/", "data/", "assets/"],
            "security_model": "Orbit browser code runs in a sandboxed iframe. Do not place credentials or session material in orbit files; authenticated retrieval must happen through authorized server-side tools before writing safe display data.",
            "available_capability_clusters": [
                "arkorbit_file_authoring",
            ],
            "description": "User is on a per-user orbit canvas where the agent can create durable HTML, JavaScript, CSS, data, and asset files rendered in a sandboxed iframe."
        }))
    }

    pub(super) async fn run_inbound_security_precheck(
        &self,
        classification_message: &str,
        stored_user_message: &str,
        channel: &str,
        conversation_key: &str,
        is_new_conversation: bool,
        project_id: Option<&str>,
        user_message_already_recorded: bool,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<InboundSecurityPrecheck> {
        // Abuse-tracker short-circuit: if this source is currently in
        // pending-approval or paused status from prior guard trips, decline
        // before any early command path can mutate state.
        let abuse_source = crate::security::abuse_tracker::SourceKey {
            channel_id: channel.to_string(),
            user_identity: None,
        };
        let abuse_tracker = crate::security::abuse_tracker::AbuseTracker::new(
            self.storage.db(),
            self.config.security.abuse_tracker.clone(),
        );
        match abuse_tracker.current_status(&abuse_source).await {
            Ok(status) if status.should_suppress_responses() => {
                let reply = match status {
                    crate::security::abuse_tracker::TrackerStatus::PendingApproval => {
                        "This channel is paused pending an operator review. Please wait - your administrator will decide whether to resume or pause further messages."
                    }
                    crate::security::abuse_tracker::TrackerStatus::Paused => {
                        "This channel has been paused by an operator. Please contact your administrator."
                    }
                    crate::security::abuse_tracker::TrackerStatus::Normal => unreachable!(),
                };
                let processed = self
                    .persist_immediate_exchange(
                        stored_user_message,
                        reply,
                        ImmediateExchangeContext {
                            channel,
                            conversation_key,
                            is_new_conversation,
                            project_id,
                            model_used: "security_guard",
                            user_message_already_recorded,
                            memory_capture_allowed: false,
                            memory_capture_source: None,
                        },
                    )
                    .await?;
                return Ok(InboundSecurityPrecheck::Respond(processed));
            }
            Err(error) => {
                tracing::warn!(
                    target: "security.abuse",
                    channel = %channel,
                    error = %error,
                    "abuse_tracker status lookup failed; continuing with inbound guard"
                );
            }
            _ => {}
        }

        // Intent-based inbound guard. The classifier sees the already-redacted
        // storage form, then normalization removes unicode obfuscation controls.
        let normalized_for_guard = crate::security::normalize_for_analysis(classification_message);
        let new_empty_conversation = is_new_conversation && !user_message_already_recorded;
        let recent_artifacts = if new_empty_conversation {
            Vec::new()
        } else {
            Self::conversation_artifacts_for_prompt(
                &self.load_recent_artifact_contexts(conversation_key).await,
                INBOUND_CLASSIFIER_RECENT_ARTIFACTS,
            )
        };
        let recent_artifacts_context = (!recent_artifacts.is_empty())
            .then(|| serde_json::Value::Array(recent_artifacts.clone()));
        let mut recent_messages_context = if new_empty_conversation {
            Vec::new()
        } else {
            self.recent_messages_for_intent_gating(conversation_key, stored_user_message)
                .await
                .into_iter()
                .rev()
                .take(4)
                .map(|message| {
                    serde_json::json!({
                        "role": message.role,
                        "content": safe_truncate(
                            &crate::security::redact_secret_input(&message.content).text,
                            360,
                        ),
                        "timestamp": message._timestamp,
                    })
                })
                .collect::<Vec<_>>()
        };
        recent_messages_context.reverse();
        let recent_messages_context_value = (!recent_messages_context.is_empty())
            .then(|| serde_json::Value::Array(recent_messages_context.clone()));
        let embedding_context = (!recent_messages_context.is_empty()
            || recent_artifacts_context.is_some())
        .then(|| {
            serde_json::json!({
                "recent_messages": recent_messages_context_value.clone(),
                "recent_actionable_artifacts": recent_artifacts_context,
            })
            .to_string()
        });
        if let Some(embedder) = self.embedding_client.as_deref() {
            match crate::security::embedding_classifier::classify_inbound_embedding_fast(
                embedder,
                &normalized_for_guard,
                embedding_context.as_deref(),
                Some(self.data_dir.as_path()),
            )
            .await
            {
                Ok(Some(fast)) => {
                    tracing::info!(
                        target: "security.inbound",
                        category = ?fast.category,
                        concept = %fast.concept,
                        score = fast.score,
                        margin = fast.margin,
                        "inbound embedding classifier accepted high-confidence fast path"
                    );
                    match &fast.decision.verdict {
                        crate::security::intent_classifier::IntentVerdict::Block {
                            message: safe_reply,
                            rule_id,
                            severity,
                        } => {
                            self.security_events.record_injection_attempt();
                            tracing::warn!(
                                target: "security.inbound",
                                rule_id = %rule_id,
                                severity = severity,
                                channel = %channel,
                                "inbound embedding classifier blocked message"
                            );
                            let source_label = inbound_security_source_label(channel);
                            let alert_msg = format!(
                                "Security guard blocked a message from {} (rule {}).",
                                &source_label, rule_id
                            );
                            self.emit_notification("Security Alert", &alert_msg, "error", "security")
                                .await;
                            self.notify_preferred_channel(&alert_msg).await;
                            match abuse_tracker.record_trip(&abuse_source).await {
                                Ok(outcome) if outcome.newly_pending => {
                                    let escalation = format!(
                                        "Security escalation: {} reached {} guard trips in the configured window. Operator approval required to resume.",
                                        &source_label, outcome.trip_count_in_window
                                    );
                                    self.emit_notification(
                                        "Security approval required",
                                        &escalation,
                                        "error",
                                        "security",
                                    )
                                    .await;
                                    self.notify_preferred_channel(&escalation).await;
                                }
                                Ok(_) => {}
                                Err(error) => {
                                    tracing::warn!(
                                        target: "security.abuse",
                                        channel = %channel,
                                        error = %error,
                                        "abuse_tracker.record_trip failed after embedding block; block applied but escalation state not updated"
                                    );
                                }
                            }
                            let processed = self
                                .persist_immediate_exchange(
                                    stored_user_message,
                                    safe_reply,
                                    ImmediateExchangeContext {
                                        channel,
                                        conversation_key,
                                        is_new_conversation,
                                        project_id,
                                        model_used: "security_embedding_guard",
                                        user_message_already_recorded,
                                        memory_capture_allowed: false,
                                        memory_capture_source: None,
                                    },
                                )
                                .await?;
                            return Ok(InboundSecurityPrecheck::Respond(processed));
                        }
                        crate::security::intent_classifier::IntentVerdict::Allow => {
                            let memory_capture_allowed =
                                fast.decision.memory_capture.should_capture;
                            let mut routing = fast.decision.routing.clone();
                            self.enrich_product_help_routing_doc_ids(
                                &mut routing,
                                &normalized_for_guard,
                            )
                            .await;
                            return Ok(InboundSecurityPrecheck::Continue {
                                memory_capture_allowed,
                                routing: Some(routing),
                                routing_trusted: true,
                                direct_response: None,
                            });
                        }
                        crate::security::intent_classifier::IntentVerdict::AllowWithUncheckedTag {
                            ..
                        } => {
                            let mut routing = fast.decision.routing.clone();
                            self.enrich_product_help_routing_doc_ids(
                                &mut routing,
                                &normalized_for_guard,
                            )
                            .await;
                            return Ok(InboundSecurityPrecheck::Continue {
                                memory_capture_allowed: false,
                                routing: Some(routing),
                                routing_trusted: false,
                                direct_response: None,
                            });
                        }
                        crate::security::intent_classifier::IntentVerdict::RouterUnavailable {
                            ..
                        } => {}
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        target: "security.inbound",
                        error = %error,
                        "inbound embedding classifier unavailable; falling back to LLM classifier"
                    );
                }
            }
        }
        let pending_actions_for_guard = self.pending_conversation_actions(conversation_key).await;
        let trusted_prior_assistant_message = if pending_actions_for_guard.is_empty() {
            None
        } else {
            self.recent_trusted_assistant_message_for_inbound_guard(
                conversation_key,
                stored_user_message,
            )
            .await
        };
        let inbound_policy = crate::security::intent_classifier::default_policy();
        let mut inbound_candidates = self.llm_candidates_for_role(&ModelRole::Fast);
        if inbound_candidates.is_empty() {
            inbound_candidates.push(self.primary_llm_candidate());
        }
        let mut inbound_candidates = self
            .reorder_candidates_with_failover(inbound_candidates, Some(conversation_key))
            .await;
        if inbound_candidates.is_empty() {
            inbound_candidates.push(self.primary_llm_candidate());
        }
        // Per-call structural surface context. The chat handler routes the
        // ArkOrbit OrbitChat panel through `channel == "arkorbit"`. When we
        // see that channel we hand the classifier a structured JSON
        // describing the surface and orbit file-authoring capability. The
        // classifier reasons from that context, never from a phrase or
        // keyword list.
        let surface_context = self.build_inbound_surface_context(channel).await;
        let mut inbound_decision = None;
        for candidate in inbound_candidates.iter().take(2) {
            let decision = crate::security::intent_classifier::classify_inbound_with_metadata(
                &candidate.client,
                &inbound_policy,
                &normalized_for_guard,
                recent_messages_context_value.as_ref(),
                trusted_prior_assistant_message.as_deref(),
                surface_context.as_ref(),
                recent_artifacts_context.as_ref(),
                stream_tx,
            )
            .await;
            if let Some(model_response) = decision.model_response.as_ref() {
                self.record_llm_usage(channel, "inbound_intent_classifier", model_response)
                    .await;
            }
            if matches!(
                decision.verdict,
                crate::security::intent_classifier::IntentVerdict::RouterUnavailable { .. }
            ) {
                tracing::warn!(
                    target: "security.inbound",
                    slot_id = %candidate.slot_id,
                    slot_label = %candidate.slot_label,
                    "inbound intent classifier candidate returned no usable routing decision"
                );
                inbound_decision = Some(decision);
                continue;
            }
            inbound_decision = Some(decision);
            break;
        }
        let inbound_decision = inbound_decision.unwrap_or_else(|| {
            crate::security::intent_classifier::InboundClassificationDecision {
                verdict: crate::security::intent_classifier::IntentVerdict::RouterUnavailable {
                    reason: "no inbound classifier model candidates available".to_string(),
                },
                memory_capture: Default::default(),
                routing: Default::default(),
                direct_response: None,
                model_response: None,
            }
        });
        let memory_capture_allowed = inbound_decision.memory_capture.should_capture;
        let mut routing = inbound_decision.routing.clone();
        self.enrich_product_help_routing_doc_ids(&mut routing, &normalized_for_guard)
            .await;

        match &inbound_decision.verdict {
            crate::security::intent_classifier::IntentVerdict::Block {
                message: safe_reply,
                rule_id,
                severity,
            } => {
                self.security_events.record_injection_attempt();
                tracing::warn!(
                    target: "security.inbound",
                    rule_id = %rule_id,
                    severity = severity,
                    channel = %channel,
                    "inbound intent classifier blocked message"
                );
                let source_label = inbound_security_source_label(channel);
                let alert_msg = format!(
                    "Security guard blocked a message from {} (rule {}).",
                    &source_label, rule_id
                );
                self.emit_notification("Security Alert", &alert_msg, "error", "security")
                    .await;
                self.notify_preferred_channel(&alert_msg).await;
                match abuse_tracker.record_trip(&abuse_source).await {
                    Ok(outcome) if outcome.newly_pending => {
                        let escalation = format!(
                            "Security escalation: {} reached {} guard trips in the configured window. Operator approval required to resume.",
                            &source_label, outcome.trip_count_in_window
                        );
                        self.emit_notification(
                            "Security approval required",
                            &escalation,
                            "error",
                            "security",
                        )
                        .await;
                        self.notify_preferred_channel(&escalation).await;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(
                            target: "security.abuse",
                            channel = %channel,
                            error = %error,
                            "abuse_tracker.record_trip failed; block applied but escalation state not updated"
                        );
                    }
                }
                let processed = self
                    .persist_immediate_exchange(
                        stored_user_message,
                        safe_reply,
                        ImmediateExchangeContext {
                            channel,
                            conversation_key,
                            is_new_conversation,
                            project_id,
                            model_used: "security_guard",
                            user_message_already_recorded,
                            memory_capture_allowed: false,
                            memory_capture_source: None,
                        },
                    )
                    .await?;
                Ok(InboundSecurityPrecheck::Respond(processed))
            }
            crate::security::intent_classifier::IntentVerdict::AllowWithUncheckedTag {
                reason,
                intent_kinds,
            } => {
                tracing::warn!(
                    target: "security.inbound",
                    reason = %reason,
                    channel = %channel,
                    "inbound intent classifier degraded; message passed with unchecked tag"
                );
                let _ = (reason, intent_kinds);
                Ok(InboundSecurityPrecheck::Continue {
                    memory_capture_allowed: false,
                    routing: Some(routing),
                    routing_trusted: false,
                    direct_response: None,
                })
            }
            crate::security::intent_classifier::IntentVerdict::RouterUnavailable { reason } => {
                tracing::warn!(
                    target: "security.inbound",
                    reason = %reason,
                    channel = %channel,
                    "inbound intent router unavailable; continuing without routing hints"
                );
                Ok(InboundSecurityPrecheck::Continue {
                    // A timed-out router must not fan out into the memory
                    // extractor for every operational/read-only request. When
                    // routing is unavailable, later direct-reply gating may
                    // still opt into semantic memory capture for conversational
                    // turns, but agent-loop/tool turns stay lean.
                    memory_capture_allowed: false,
                    routing: None,
                    routing_trusted: false,
                    direct_response: None,
                })
            }
            crate::security::intent_classifier::IntentVerdict::Allow => {
                Ok(InboundSecurityPrecheck::Continue {
                    memory_capture_allowed,
                    routing: Some(routing),
                    routing_trusted: true,
                    direct_response: inbound_decision.direct_response.clone(),
                })
            }
        }
    }

    pub(super) async fn persist_immediate_exchange(
        &self,
        message: &str,
        response: &str,
        context: ImmediateExchangeContext<'_>,
    ) -> Result<ProcessedMessage> {
        let trace_id = uuid::Uuid::new_v4().to_string();
        let trace_time = chrono::Utc::now();
        let filtered_response = self.security.filter_output(response);
        if !filtered_response.redactions.is_empty() {
            tracing::warn!(
                "Security: redacted sensitive data from immediate output before persistence ({} rule match(es))",
                filtered_response.redactions.len()
            );
        }
        let safe_response = filtered_response.text;
        let trace_ref = Arc::new(RwLock::new(ExecutionTrace {
            id: trace_id.clone(),
            message: message.to_string(),
            channel: context.channel.to_string(),
            started_at: Some(trace_time),
            completed_at: Some(trace_time),
            steps: vec![
                ExecutionStep {
                    icon: "[fast]".to_string(),
                    title: "Message Received".to_string(),
                    detail: format!(
                        "Immediate reply path | Channel: {} | Length: {} chars",
                        context.channel,
                        message.chars().count()
                    ),
                    step_type: "info".to_string(),
                    data: None,
                    timestamp: trace_time,
                    duration_ms: Some(0),
                },
                ExecutionStep {
                    icon: "[reply]".to_string(),
                    title: "Immediate Response".to_string(),
                    detail: format!(
                        "Returned without the full tool loop using {}.",
                        context.model_used
                    ),
                    step_type: "success".to_string(),
                    data: Some(safe_truncate(&safe_response, 8000)),
                    timestamp: trace_time,
                    duration_ms: Some(0),
                },
            ],
            proof_id: None,
            response: Some(safe_response.clone()),
            model: Some(context.model_used.to_string()),
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cost_usd: 0.0,
            complexity: Some("immediate".to_string()),
            plan: None,
        }));
        let trace_snapshot = trace_ref.read().await.clone();
        self.remember_completed_trace_snapshot(trace_snapshot.clone(), true)
            .await;
        tracing::info!(
            "Request started: trace={} channel={} flow=immediate resumed={}",
            trace_id,
            context.channel,
            context.user_message_already_recorded
        );
        tracing::info!(
            "Request completed: trace={} channel={} status=completed duration=0ms tools=0",
            trace_id,
            context.channel
        );

        // Mirror normal chat persistence path for immediate shortcut responses.
        {
            let mut history = self.conversation_history.write().await;
            let conversation_history = history
                .entry(context.conversation_key.to_string())
                .or_insert_with(Vec::new);
            if !context.user_message_already_recorded {
                conversation_history.push(ConversationMessage {
                    role: "user".to_string(),
                    content: message.to_string(),
                    _timestamp: chrono::Utc::now(),
                });
            }
            conversation_history.push(ConversationMessage {
                role: "assistant".to_string(),
                content: safe_response.clone(),
                _timestamp: chrono::Utc::now(),
            });
            if conversation_history.len() > 10 {
                conversation_history.drain(0..conversation_history.len() - 10);
            }
        }

        let mut conversation_title: Option<String> = None;
        if !context.conversation_key.is_empty() {
            if context.is_new_conversation {
                let title = self.generate_conversation_title(message);
                *self.last_conversation_title.write().await = Some(title.clone());
                conversation_title = Some(title);
            } else {
                *self.last_conversation_title.write().await = None;
            }
        }

        *self.last_conversation_id.write().await = Some(context.conversation_key.to_string());

        let user_outcome = self
            .build_response_heuristic_outcome(&safe_response, &[], &[], None)
            .unwrap_or_else(|| {
                self.execution_supervisor
                    .build_success_outcome(&safe_response, &[], &[])
            });
        let run_status = Self::execution_run_status_for_outcome(&user_outcome);
        self.spawn_deferred_exchange_persistence(DeferredExchangePersistence {
            kind: DeferredExchangePersistenceKind::Immediate,
            trace_snapshot,
            message: message.to_string(),
            response: safe_response.clone(),
            channel: context.channel.to_string(),
            conversation_key: context.conversation_key.to_string(),
            project_id: context.project_id.map(str::to_string),
            model_used: context.model_used.to_string(),
            user_message_already_recorded: context.user_message_already_recorded,
            memory_capture_allowed: context.memory_capture_allowed,
            memory_capture_source: context.memory_capture_source.map(str::to_string),
            user_message_id: uuid::Uuid::new_v4().to_string(),
            assistant_message_id: uuid::Uuid::new_v4().to_string(),
            user_timestamp: chrono::Utc::now().to_rfc3339(),
            assistant_timestamp: chrono::Utc::now().to_rfc3339(),
            is_new_conversation: context.is_new_conversation,
            conversation_title: conversation_title.clone(),
            user_outcome: user_outcome.clone(),
        });

        Ok(ProcessedMessage {
            response: safe_response,
            conversation_id: Some(context.conversation_key.to_string()),
            conversation_title,
            run_id: None,
            run_status: Some(run_status.as_str().to_string()),
            trace_id: None,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            choices: Vec::new(),
            degradation: Vec::new(),
            attempted_models: Vec::new(),
            user_outcome: Some(user_outcome),
            trace_steps: Vec::new(),
            turn_records: Vec::new(),
            turn_plan: None,
        })
    }

    pub(crate) async fn persist_completed_trace(&self, trace_ref: &Arc<RwLock<ExecutionTrace>>) {
        let trace_snapshot = trace_ref.read().await.clone();
        if trace_snapshot.id.trim().is_empty() {
            return;
        }

        self.remember_completed_trace_snapshot(
            trace_snapshot.clone(),
            !Arc::ptr_eq(trace_ref, &self.last_trace),
        )
        .await;

        if let Err(e) = self
            .persist_completed_trace_snapshot_durable(&trace_snapshot)
            .await
        {
            tracing::warn!(
                "Failed to persist execution trace '{}': {}",
                trace_snapshot.id,
                e
            );
        } else {
            self.record_completed_interaction_for_self_tune().await;
        }
    }

    async fn persist_completed_trace_snapshot_durable(
        &self,
        trace_snapshot: &ExecutionTrace,
    ) -> Result<()> {
        self.encrypted_storage
            .insert_execution_trace_encrypted(trace_snapshot)
            .await?;
        let observability_endpoint = crate::core::observability::normalize_observability_endpoint(
            &self.config.observability.provider,
            &self.config.observability.endpoint,
        );
        let observability_ready = self.config.observability.enabled
            && !observability_endpoint.is_empty()
            && crate::core::observability::has_observability_auth_token(
                &self.config_dir,
                Some(&self.data_dir),
            )
            .unwrap_or(false);
        if observability_ready {
            let provider = crate::core::observability::normalize_observability_provider(
                &self.config.observability.provider,
            );
            match crate::core::observability::export_execution_trace(
                &self.config,
                &self.config_dir,
                &self.data_dir,
                &self.storage,
                trace_snapshot,
                "trace_completed",
            )
            .await
            {
                Ok(()) => {
                    tracing::info!(
                        "Observability: exported trace '{}' to {}",
                        trace_snapshot.id,
                        provider
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "Observability: export failed for trace '{}' to {}: {}",
                        trace_snapshot.id,
                        provider,
                        e
                    );
                }
            }
        }

        Ok(())
    }

    async fn record_completed_interaction_for_self_tune(&self) {
        // Self-tune: track interaction for adaptive learning
        crate::core::self_tune::on_interaction_completed(
            &self.storage,
            &self.encrypted_storage,
            &self.llm,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct_state() -> DirectConversationRuntimeState {
        DirectConversationRuntimeState {
            routing_trusted: true,
            supported_surface: true,
            ..Default::default()
        }
    }

    fn routing_signal(
        should_execute: bool,
        tool_use_expected: bool,
        current_answer_expected: bool,
        durable_work_expected: bool,
        multi_goal: bool,
        goal_durabilities: &[&str],
    ) -> crate::security::intent_classifier::InboundRoutingSignal {
        crate::security::intent_classifier::InboundRoutingSignal {
            should_execute,
            tool_use_expected,
            multi_goal,
            durable_work_expected,
            current_answer_expected,
            semantic_queries: vec!["Respond conversationally".to_string()],
            required_capabilities: vec!["Direct text response".to_string()],
            rationale: Some("No execution is required.".to_string()),
            saved_user_facts_expected: false,
            product_help_expected: false,
            live_state_expected: false,
            external_info_expected: false,
            profile_lookup_kind: None,
            grounding_doc_ids: Vec::new(),
            goals: goal_durabilities
                .iter()
                .enumerate()
                .map(|(index, durability)| {
                    let durable = durability.trim() != "none";
                    crate::security::intent_classifier::InboundTurnGoal {
                        id: format!("g{}", index + 1),
                        intent_summary: "Provide a direct response".to_string(),
                        capability_query: "Direct text response".to_string(),
                        expected_outcome: "A concise answer in the current chat turn".to_string(),
                        durability: durability.to_string(),
                        side_effect: if durable || should_execute || tool_use_expected {
                            "write".to_string()
                        } else {
                            "none".to_string()
                        },
                        dependencies: Vec::new(),
                        ..Default::default()
                    }
                })
                .collect(),
        }
    }

    fn memory_item(
        id: &str,
        memory_kind: &str,
        value: &str,
        sensitivity: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
    ) -> crate::storage::experience_item::Model {
        crate::storage::experience_item::Model {
            id: id.to_string(),
            kind: memory_kind.to_string(),
            scope: if conversation_id.is_some() {
                "conversation".to_string()
            } else if project_id.is_some() {
                "project".to_string()
            } else {
                "global".to_string()
            },
            project_id: project_id.map(str::to_string),
            conversation_id: conversation_id.map(str::to_string),
            title: format!("Saved {}", memory_kind),
            content: value.to_string(),
            normalized_key: id.to_string(),
            confidence: 0.95,
            support_count: 1,
            contradiction_count: 0,
            status: "active".to_string(),
            metadata: serde_json::json!({
                "memory_kind": memory_kind,
                "sensitivity": sensitivity,
                "durability": "permanent",
            }),
            last_supported_at: None,
            last_contradicted_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            embedding: None,
        }
    }

    fn visual_attachment_hints() -> RequestExecutionHints {
        RequestExecutionHints {
            attachments: vec![ChatAttachmentHint {
                upload_id: "11111111-1111-1111-1111-111111111111".to_string(),
                kind: "visual".to_string(),
                content_type: Some("image/png".to_string()),
                document_id: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn empty_visual_attachment_can_seed_memory_capture_from_analysis() {
        let source = attachment_only_visual_memory_capture_source(
            "",
            "The screenshot shows a preference for dense, terminal-like dark UI.",
            &visual_attachment_hints(),
        )
        .expect("empty visual-only turn should provide a memory analysis source");

        assert!(source.contains("visual-only user turn"));
        assert!(source.contains("dense, terminal-like dark UI"));
    }

    #[test]
    fn visual_attachment_with_message_does_not_switch_to_preference_capture() {
        let source = attachment_only_visual_memory_capture_source(
            "Fix the layout shown here.",
            "The screenshot shows a dense, terminal-like dark UI.",
            &visual_attachment_hints(),
        );

        assert!(source.is_none());
    }

    #[test]
    fn chat_storage_preserves_contact_info_but_redacts_high_risk_pii() {
        let stored = redact_chat_message_for_storage(
            "Email me at user@example.com, call 555-123-4567, ssn 123-45-6789, card 4111 1111 1111 1111, host 192.168.1.100",
        );

        assert!(stored.contains("user@example.com"));
        assert!(stored.contains("555-123-4567"));
        assert!(stored.contains("192.168.1.100"));
        assert!(stored.contains("[SSN]"));
        assert!(stored.contains("[CARD]"));
    }

    #[test]
    fn contact_info_can_trigger_semantic_memory_capture_without_phrase_rules() {
        assert!(has_contact_info_for_memory_capture(
            "Reach me at user@example.com"
        ));
        assert!(has_contact_info_for_memory_capture("555-123-4567"));
        assert!(!has_contact_info_for_memory_capture("SSN 123-45-6789"));
        assert!(!has_contact_info_for_memory_capture("server 192.168.1.100"));
    }

    #[test]
    fn direct_conversation_allows_semantic_no_tool_routing() {
        let routing = routing_signal(false, false, true, false, false, &["none"]);

        assert!(should_use_direct_conversation_path(
            Some(&routing),
            direct_state()
        ));
        assert_eq!(
            turn_execution_path_from_routing(Some(&routing), direct_state()),
            TurnExecutionPath::DirectReply
        );
    }

    #[test]
    fn direct_decline_neutralizes_stale_direct_reply_routing() {
        let mut routing = routing_signal(false, false, true, false, false, &["none"]);

        assert!(neutralize_direct_reply_routing_after_direct_decline(
            Some(&mut routing),
            direct_state(),
            None,
            "Inspect the recent run and tell me what failed",
            None
        ));
        assert!(routing.current_answer_expected);
        assert!(routing.should_execute);
        assert!(routing.tool_use_expected);
        assert_eq!(
            turn_execution_path_from_routing(Some(&routing), direct_state()),
            TurnExecutionPath::AgentLoop
        );
    }

    #[test]
    fn direct_decline_preserves_read_only_external_info_routing() {
        let mut routing = routing_signal(false, false, true, false, false, &["none"]);

        assert!(neutralize_direct_reply_routing_after_direct_decline(
            Some(&mut routing),
            direct_state(),
            Some(DirectConversationDeclineKind::ExternalInfo),
            "Find the latest release notes for the SDK",
            Some("Needs public current information.")
        ));
        assert!(routing.current_answer_expected);
        assert!(routing.should_execute);
        assert!(routing.tool_use_expected);
        assert!(routing.external_info_expected);
        assert!(!routing.durable_work_expected);
        assert_eq!(routing.semantic_queries.len(), 1);
        assert!(routing.semantic_queries[0].contains("Find the latest release notes"));
        assert_eq!(
            routing.required_capabilities,
            vec![
                "external public information lookup for current user request: Find the latest release notes for the SDK"
                    .to_string()
            ]
        );
        assert!(
            routing
                .rationale
                .as_deref()
                .unwrap_or_default()
                .contains("Needs public current information")
        );
    }

    #[test]
    fn direct_conversation_uses_routerless_probe_when_classifier_degrades() {
        let routing = routing_signal(false, false, true, false, false, &["none"]);
        let state = DirectConversationRuntimeState {
            routing_trusted: false,
            ..direct_state()
        };

        assert!(!should_use_direct_conversation_path(Some(&routing), state));
        assert!(should_use_direct_conversation_path(None, state));
        assert_eq!(
            turn_execution_path_from_routing(None, state),
            TurnExecutionPath::DirectReply
        );
    }

    #[test]
    fn direct_conversation_blocks_execution_and_tool_work() {
        let execute = routing_signal(true, false, true, false, false, &["none"]);
        let tool = routing_signal(false, true, true, false, false, &["none"]);

        assert!(!should_use_direct_conversation_path(
            Some(&execute),
            direct_state()
        ));
        assert!(!should_use_direct_conversation_path(
            Some(&tool),
            direct_state()
        ));
    }

    #[test]
    fn direct_conversation_blocks_durable_or_multi_goal_work() {
        let durable_goal = routing_signal(false, false, true, false, false, &["deployment"]);
        let multiple_goals = routing_signal(false, false, true, false, false, &["none", "none"]);

        for signal in [durable_goal, multiple_goals] {
            assert!(!should_use_direct_conversation_path(
                Some(&signal),
                direct_state()
            ));
        }
    }

    #[test]
    fn mixed_social_and_app_work_routing_cannot_use_direct_reply_path() {
        let mut mixed = routing_signal(true, true, true, true, false, &["deployment"]);
        mixed.semantic_queries =
            vec!["Friendly conversational opening plus requested browser app delivery".to_string()];
        mixed.required_capabilities =
            vec!["Generate and host a persistent runnable application".to_string()];

        assert_eq!(
            turn_execution_path_from_routing(Some(&mixed), direct_state()),
            TurnExecutionPath::AgentLoop
        );
        assert!(!should_use_direct_conversation_path(
            Some(&mixed),
            direct_state()
        ));
    }

    #[test]
    fn direct_conversation_blocks_runtime_state_that_needs_full_loop() {
        let routing = routing_signal(false, false, true, false, false, &["none"]);

        for state in [
            DirectConversationRuntimeState {
                has_attachments: true,
                ..direct_state()
            },
            DirectConversationRuntimeState {
                has_secret_offered: true,
                ..direct_state()
            },
            DirectConversationRuntimeState {
                has_pending_actions: true,
                ..direct_state()
            },
            DirectConversationRuntimeState {
                has_pending_credential_prompt: true,
                ..direct_state()
            },
            DirectConversationRuntimeState {
                user_message_already_recorded: true,
                ..direct_state()
            },
            DirectConversationRuntimeState {
                skip_inbound_security_precheck: true,
                ..direct_state()
            },
            DirectConversationRuntimeState {
                supported_surface: false,
                ..direct_state()
            },
        ] {
            assert!(!should_use_direct_conversation_path(Some(&routing), state));
        }
    }

    #[test]
    fn memory_capture_signal_keeps_canonical_conversational_goal_shape_direct() {
        let routing = routing_signal(false, false, true, false, false, &["none"]);

        assert!(should_use_direct_conversation_path(
            Some(&routing),
            direct_state()
        ));
        assert_eq!(
            turn_execution_path_from_routing(Some(&routing), direct_state()),
            TurnExecutionPath::DirectReply
        );
    }

    #[test]
    fn non_conversational_work_does_not_use_direct_reply_path() {
        let mut saved_lookup = routing_signal(false, false, true, false, false, &["none"]);
        saved_lookup.saved_user_facts_expected = true;
        saved_lookup.goals[0].groundings = vec!["user_memory".to_string()];
        let mut external = routing_signal(false, false, true, false, false, &["none"]);
        external.external_info_expected = true;
        external.goals[0].groundings = vec!["external_info".to_string()];
        let tool = routing_signal(false, true, true, false, false, &["none"]);
        let execute = routing_signal(true, false, true, false, false, &["none"]);

        for signal in [saved_lookup, external, tool, execute] {
            assert_eq!(
                turn_execution_path_from_routing(Some(&signal), direct_state()),
                TurnExecutionPath::AgentLoop
            );
        }
    }

    #[test]
    fn transient_read_only_lookup_does_not_need_speculative_memory_probe() {
        let mut external = routing_signal(true, true, true, false, false, &["none"]);
        external.external_info_expected = true;
        external.goals[0].groundings = vec!["external_info".to_string()];
        external.goals[0].side_effect = "none".to_string();
        let mut live_state = routing_signal(true, true, true, false, false, &["none"]);
        live_state.live_state_expected = true;
        live_state.goals[0].groundings = vec!["local_state".to_string()];
        live_state.goals[0].side_effect = "none".to_string();
        let mut saved_lookup = external.clone();
        saved_lookup.saved_user_facts_expected = true;
        saved_lookup.goals[0].groundings = vec!["user_memory".to_string()];

        assert!(routing_is_transient_read_only_lookup(Some(&external)));
        assert!(routing_is_transient_read_only_lookup(Some(&live_state)));
        assert!(!routing_is_transient_read_only_lookup(Some(&saved_lookup)));
    }

    #[test]
    fn speculative_memory_probe_does_not_run_without_router_memory_signal() {
        assert!(!should_enqueue_semantic_user_memory_capture(
            "I prefer concise status updates.",
            direct_state(),
            TurnExecutionPath::DirectReply
        ));
        assert!(!should_enqueue_semantic_user_memory_capture(
            "what current apps do i have",
            direct_state(),
            TurnExecutionPath::AgentLoop
        ));
    }

    #[test]
    fn direct_conversation_legacy_json_output_still_parses_for_fallback() {
        let fallback = extract_direct_conversation_json_object(
            r#"{"can_answer_directly":false,"answer":"","rationale":"needs tools"}"#,
        )
        .and_then(|value| serde_json::from_value::<DirectConversationModelOutput>(value).ok())
        .expect("fallback JSON should parse");
        assert!(!fallback.can_answer_directly);
        assert!(fallback.answer.is_empty());
        assert!(fallback.decline_kind.is_none());
    }

    #[test]
    fn direct_conversation_prompt_uses_structured_decline_contract() {
        let prompt = direct_conversation_system_prompt();

        assert!(prompt.contains("\"can_answer_directly\":false"));
        assert!(prompt.contains("\"decline_kind\":\"external_info\""));
        assert!(prompt.contains("external_info, live_state, product_help"));
        assert!(prompt.contains("set can_answer_directly=false"));
        assert!(prompt.contains("Do not state or imply persistence"));
        assert!(!prompt.contains("say that this needs the full agent loop"));
    }

    #[test]
    fn direct_local_answer_mode_query_uses_routing_signal_not_raw_message() {
        let routing = crate::security::intent_classifier::InboundRoutingSignal {
            semantic_queries: vec![
                "Recall the immediately previous user question in this conversation".to_string(),
            ],
            goals: vec![crate::security::intent_classifier::InboundTurnGoal {
                id: "g1".to_string(),
                intent_summary: "Identify the previous user turn".to_string(),
                capability_query: "Visible conversation history lookup".to_string(),
                expected_outcome: "Answer with the prior user message".to_string(),
                durability: "none".to_string(),
                dependencies: Vec::new(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let query = direct_local_answer_mode_query(&routing).expect("routing query");
        assert!(query.contains("previous user question"));
        assert!(query.contains("prior user message"));
    }

    #[test]
    fn direct_conversation_plain_refusal_is_not_structured_direct_answer() {
        let parsed =
            extract_direct_conversation_json_object("I cannot use live tools from this path.")
                .and_then(|value| {
                    serde_json::from_value::<DirectConversationModelOutput>(value).ok()
                });

        assert!(parsed.is_none());
    }

    #[test]
    fn direct_conversation_prompt_includes_recent_actionable_artifacts() {
        let recent_artifacts = vec![serde_json::json!({
            "artifact_type": "app",
            "artifact_id": "838430cf",
            "title": "Public Webcam Monitor",
            "related_actions": ["ark_inspect", "file_write", "app_restart"]
        })];
        let prompt = direct_conversation_user_prompt(
            "the page keeps refreshing with no stable camera feed",
            "conversation-1",
            &[],
            &recent_artifacts,
            None,
        );
        let value: serde_json::Value = serde_json::from_str(&prompt).expect("prompt json");
        assert_eq!(
            value["recent_actionable_artifacts"][0]["title"],
            "Public Webcam Monitor"
        );
        assert_eq!(
            value["recent_actionable_artifacts"][0]["related_actions"][2],
            "app_restart"
        );
    }

    #[test]
    fn direct_memory_answer_returns_single_safe_identity() {
        let items = vec![memory_item(
            "memory-1",
            "identity",
            "user_name: Mira",
            "personal_identifier",
            None,
            None,
        )];

        let answer = select_direct_memory_answer(
            &items,
            Some("identity"),
            None,
            Some("conversation-1"),
            chrono::Utc::now(),
        )
        .expect("safe identity memory should answer directly");

        assert!(answer.contains("Mira"));
    }

    #[test]
    fn direct_memory_answer_rejects_sensitive_memory() {
        let items = vec![memory_item(
            "memory-1",
            "identity",
            "private_detail: sensitive value",
            "sensitive",
            None,
            None,
        )];

        assert!(
            select_direct_memory_answer(
                &items,
                Some("identity"),
                None,
                Some("conversation-1"),
                chrono::Utc::now(),
            )
            .is_none()
        );
    }

    #[test]
    fn direct_memory_answer_rejects_conflicting_same_scope_values() {
        let items = vec![
            memory_item(
                "memory-1",
                "identity",
                "user_name: Mira",
                "personal_identifier",
                None,
                None,
            ),
            memory_item(
                "memory-2",
                "identity",
                "user_name: Robin",
                "personal_identifier",
                None,
                None,
            ),
        ];

        assert!(
            select_direct_memory_answer(
                &items,
                Some("identity"),
                None,
                Some("conversation-1"),
                chrono::Utc::now(),
            )
            .is_none()
        );
    }

    #[test]
    fn direct_memory_answer_prefers_more_specific_scope() {
        let items = vec![
            memory_item(
                "global-memory",
                "identity",
                "user_name: Mira",
                "personal_identifier",
                None,
                None,
            ),
            memory_item(
                "conversation-memory",
                "identity",
                "user_name: Robin",
                "personal_identifier",
                None,
                Some("conversation-1"),
            ),
        ];

        let answer = select_direct_memory_answer(
            &items,
            Some("identity"),
            None,
            Some("conversation-1"),
            chrono::Utc::now(),
        )
        .expect("specific scoped memory should answer directly");

        assert!(answer.contains("Robin"));
        assert!(!answer.contains("Mira"));
    }
}
