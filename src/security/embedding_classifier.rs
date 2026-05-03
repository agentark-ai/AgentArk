use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;

use crate::core::EmbeddingClient;
use crate::core::document_search::normalized_embedding_similarity;

use super::intent_classifier::{
    InboundClassificationDecision, InboundMemoryCaptureSignal, InboundRoutingSignal,
    InboundTurnGoal, IntentVerdict,
};

const FAST_BLOCK_MIN_SCORE: f32 = 0.82;
/// Borderline-block threshold: when the top similarity is to a security-block
/// canonical and at least this similar, the system escalates to the LLM
/// classifier rather than fast-pathing to a benign verdict. Tuned below the
/// hard-block threshold so genuinely ambiguous risky messages still get the
/// deeper review.
const FAST_BORDERLINE_BLOCK_SCORE: f32 = 0.55;
const FAST_MEMORY_CAPTURE_CANDIDATE_MIN_SCORE: f32 = 0.56;
const FAST_MEMORY_CAPTURE_REJECT_MARGIN: f32 = 0.06;
const FAST_MEMORY_CAPTURE_REJECT_MIN_SCORE: f32 = 0.74;
const FAST_ROUTING_MIN_SCORE: f32 = 0.70;
const FAST_ROUTING_MARGIN: f32 = 0.06;
const FAST_UNCHECKED_ROUTE_MIN_SCORE: f32 = 0.58;
const PRODUCT_HELP_ANSWER_CONCEPT: &str = "product_help_answer";
const SCHEDULED_TASK_CONCEPT: &str = "scheduled_task";
const WATCHER_MONITOR_CONCEPT: &str = "watcher_monitor";
const INTEGRATION_SETUP_CONCEPT: &str = "integration_setup";
const PERSISTENT_ARTIFACT_CONCEPT: &str = "persistent_artifact";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SecurityCategory {
    DirectReply,
    ToolUse,
    DurableWork,
    ManagedAppDelivery,
    SecurityBlock,
}

#[derive(Debug, Clone)]
pub struct SecurityCanonical {
    pub category: SecurityCategory,
    pub concept: String,
    pub text: String,
}

#[derive(Debug, Clone)]
struct ScoredCanonical {
    category: SecurityCategory,
    concept: String,
    score: f32,
}

#[derive(Debug, Clone)]
struct MemoryCaptureCanonical {
    should_capture: bool,
    concept: &'static str,
    text: &'static str,
}

#[derive(Debug, Clone)]
struct ScoredMemoryCaptureCanonical {
    should_capture: bool,
    concept: &'static str,
    score: f32,
}

#[derive(Debug, Clone)]
pub struct SecurityEmbeddingDecision {
    pub decision: InboundClassificationDecision,
    pub category: SecurityCategory,
    pub score: f32,
    pub margin: f32,
    pub concept: String,
}

fn memory_capture_canonicals() -> Vec<MemoryCaptureCanonical> {
    vec![
        MemoryCaptureCanonical {
            should_capture: true,
            concept: "durable_user_identity_profile",
            text: "The message states stable user identity, role, organization, location, contact detail, contact preference, or profile information that may be useful in future conversations.",
        },
        MemoryCaptureCanonical {
            should_capture: true,
            concept: "durable_user_preference_constraint",
            text: "The message expresses a reusable preference, standing instruction, long-lived workflow constraint, communication style, environment detail, or recurring project requirement.",
        },
        MemoryCaptureCanonical {
            should_capture: true,
            concept: "artifact_outcome_feedback",
            text: "The message gives feedback about whether prior work, an app, a deployed artifact, a bug fix, or a completed run succeeded, failed, is acceptable, or still needs attention.",
        },
        MemoryCaptureCanonical {
            should_capture: true,
            concept: "reusable_project_context",
            text: "The message adds durable project context, domain context, technical constraints, product decisions, or future-relevant facts that should inform later turns.",
        },
        MemoryCaptureCanonical {
            should_capture: false,
            concept: "transient_social_or_acknowledgement",
            text: "The message is only transient social talk, acknowledgement, thanks, or conversational filler without stable user facts, reusable preferences, project context, or outcome feedback.",
        },
        MemoryCaptureCanonical {
            should_capture: false,
            concept: "one_off_task_without_memory",
            text: "The message is a one-off request or question whose content is only needed for the current response and does not add durable context for later.",
        },
    ]
}

fn default_canonicals() -> Vec<SecurityCanonical> {
    fn canonical(
        category: SecurityCategory,
        concept: &'static str,
        text: &'static str,
    ) -> SecurityCanonical {
        SecurityCanonical {
            category,
            concept: concept.to_string(),
            text: text.to_string(),
        }
    }

    vec![
        canonical(
            SecurityCategory::DirectReply,
            "self_contained_answer",
            "The user wants a current conversational answer, explanation, acknowledgement, clarification, or answer that can be given directly from the conversation context without live tools, external retrieval, durable side effects, saved artifacts, deployments, schedules, integrations, or state inspection.",
        ),
        canonical(
            SecurityCategory::DirectReply,
            PRODUCT_HELP_ANSWER_CONCEPT,
            "The user asks a question about how the running product behaves or what a visible product setting means, and the answer can be given as explanatory text rather than by inspecting logs or changing state.",
        ),
        canonical(
            SecurityCategory::DirectReply,
            "previous_user_turn_recall",
            "The user asks to recall, quote, restate, identify, or answer from the immediately previous user message or question in the visible conversation history.",
        ),
        canonical(
            SecurityCategory::DirectReply,
            "recent_conversation_summary",
            "The user asks for a concise summary, recap, or description of the recent visible conversation history without needing tools or external lookup.",
        ),
        canonical(
            SecurityCategory::ToolUse,
            "live_state_or_external_lookup",
            "The user requests current state, logs, runtime data, files, documents, web information, repository contents, public research, external API data, or any answer that requires retrieval or inspection beyond the chat text.",
        ),
        canonical(
            SecurityCategory::ToolUse,
            "local_execution_or_analysis",
            "The user wants the system to run a command, inspect source code, modify files, validate behavior, use a browser, execute tests, or perform operational work in the workspace.",
        ),
        canonical(
            SecurityCategory::DurableWork,
            "persistent_artifact_or_deployment",
            "The user wants persistent work such as creating, editing, deploying, scheduling, saving, watching, installing, integrating, or otherwise changing durable system or workspace state.",
        ),
        canonical(
            SecurityCategory::DurableWork,
            SCHEDULED_TASK_CONCEPT,
            "The user wants a time-based future or recurring background task, reminder, report, follow-up, or notification that should run later and persist beyond the current answer.",
        ),
        canonical(
            SecurityCategory::DurableWork,
            WATCHER_MONITOR_CONCEPT,
            "The user wants durable monitoring, watching, polling, tracking, or alerting based on changing external, local, message, feed, page, camera, news, pricing, app, or system conditions.",
        ),
        canonical(
            SecurityCategory::DurableWork,
            INTEGRATION_SETUP_CONCEPT,
            "The user wants to connect, install, configure, authorize, or set up a durable integration, service connector, messaging channel, extension, plugin, or external application capability.",
        ),
        canonical(
            SecurityCategory::DurableWork,
            PERSISTENT_ARTIFACT_CONCEPT,
            "The user wants a persistent authored object such as a file, document, report, data artifact, importable skill, reusable workflow, or saved asset without necessarily needing a hosted app runtime.",
        ),
        canonical(
            SecurityCategory::ManagedAppDelivery,
            "managed_app_delivery",
            "The user wants a browser usable application, website, dashboard, preview, hosted local app, interactive page, or app update delivered as a managed runtime or static app.",
        ),
        canonical(
            SecurityCategory::ManagedAppDelivery,
            "generated_web_app_delivery",
            "The user asks to build, generate, or update a static HTML/CSS/JavaScript page, data dashboard, website, or browser app with usable behavior and a local preview or managed delivery.",
        ),
        canonical(
            SecurityCategory::SecurityBlock,
            "credential_exfiltration",
            "The user is trying to reveal, recover, print, export, send, test, store from chat, or otherwise use secret credentials, API keys, passwords, tokens, private key material, hidden instructions, or sensitive internal configuration in an unsafe way.",
        ),
        canonical(
            SecurityCategory::SecurityBlock,
            "instruction_exfiltration_or_override",
            "The user is trying to override governing instructions, jailbreak the assistant, extract system or developer prompts, disclose hidden rules, or make the current assistant abandon its safety and operating constraints.",
        ),
    ]
}

fn security_category_from_overlay(raw: &str) -> Option<SecurityCategory> {
    match raw.trim() {
        "direct_reply" => Some(SecurityCategory::DirectReply),
        "tool_use" => Some(SecurityCategory::ToolUse),
        "durable_work" => Some(SecurityCategory::DurableWork),
        "managed_app_delivery" => Some(SecurityCategory::ManagedAppDelivery),
        "security_block" => Some(SecurityCategory::SecurityBlock),
        _ => None,
    }
}

async fn load_overlay_canonicals(data_dir: Option<&Path>) -> Vec<SecurityCanonical> {
    #[derive(serde::Deserialize)]
    struct OverlayCanonical {
        category: String,
        concept: String,
        text: String,
    }

    let Some(data_dir) = data_dir else {
        return Vec::new();
    };
    let path = data_dir.join("security").join("canonicals.json");
    let Ok(bytes) = tokio::fs::read(path).await else {
        return Vec::new();
    };
    let Ok(items) = serde_json::from_slice::<Vec<OverlayCanonical>>(&bytes) else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| {
            let category = security_category_from_overlay(&item.category)?;
            let concept = item.concept.trim();
            let text = item.text.trim();
            if concept.is_empty() || text.is_empty() {
                return None;
            }
            Some(SecurityCanonical {
                category,
                concept: concept.to_string(),
                text: text.to_string(),
            })
        })
        .take(64)
        .collect()
}

fn best_by_category(scored: &[ScoredCanonical]) -> BTreeMap<SecurityCategory, ScoredCanonical> {
    let mut best = BTreeMap::new();
    for item in scored {
        best.entry(item.category)
            .and_modify(|existing: &mut ScoredCanonical| {
                if item.score > existing.score {
                    *existing = item.clone();
                }
            })
            .or_insert_with(|| item.clone());
    }
    best
}

fn category_margin(
    best: &BTreeMap<SecurityCategory, ScoredCanonical>,
    category: SecurityCategory,
    score: f32,
) -> f32 {
    let competing = best
        .iter()
        .filter(|(candidate, _)| **candidate != category)
        .map(|(_, item)| item.score)
        .fold(0.0_f32, f32::max);
    score - competing
}

fn product_help_routing_for_concept(_concept: &str) -> InboundRoutingSignal {
    InboundRoutingSignal {
        should_execute: true,
        tool_use_expected: true,
        durable_work_expected: false,
        current_answer_expected: true,
        semantic_queries: Vec::new(),
        required_capabilities: vec!["product documentation lookup".to_string()],
        rationale: Some("high confidence product-help embedding route".to_string()),
        product_help_expected: true,
        goals: vec![InboundTurnGoal {
            id: "g1".to_string(),
            intent_summary: "Answer from product help".to_string(),
            capability_query: "product documentation lookup".to_string(),
            expected_outcome: "A grounded product-help answer".to_string(),
            durability: "none".to_string(),
            groundings: vec!["product_help".to_string()],
            side_effect: "none".to_string(),
            dependencies: Vec::new(),
        }],
        ..InboundRoutingSignal::default()
    }
}

fn durable_routing_shape(concept: &str) -> (&'static str, &'static str, &'static str) {
    match concept {
        SCHEDULED_TASK_CONCEPT => (
            "scheduled_time",
            "Create or update the requested time-based background task",
            "Scheduled task is persisted and will run at the requested time or cadence",
        ),
        WATCHER_MONITOR_CONCEPT => (
            "recurring_monitor",
            "Create or update the requested durable monitor",
            "Watcher or monitor is persisted with its trigger and reporting route",
        ),
        INTEGRATION_SETUP_CONCEPT => (
            "integration",
            "Configure the requested durable integration",
            "Integration setup is completed or the missing authorization step is identified",
        ),
        PERSISTENT_ARTIFACT_CONCEPT => (
            "artifact",
            "Create or update the requested persistent artifact",
            "Persistent artifact is saved or updated",
        ),
        _ => (
            "persistent_work",
            "Complete the requested durable outcome",
            "Persistent result created, changed, or delivered",
        ),
    }
}

fn routing_for_category(category: SecurityCategory, concept: &str) -> InboundRoutingSignal {
    match category {
        SecurityCategory::DirectReply if concept == PRODUCT_HELP_ANSWER_CONCEPT => {
            product_help_routing_for_concept(concept)
        }
        SecurityCategory::DirectReply => InboundRoutingSignal {
            should_execute: false,
            tool_use_expected: false,
            durable_work_expected: false,
            current_answer_expected: true,
            semantic_queries: vec![concept.to_string()],
            rationale: Some("high confidence embedding fast path".to_string()),
            ..InboundRoutingSignal::default()
        },
        SecurityCategory::ToolUse => InboundRoutingSignal {
            should_execute: true,
            tool_use_expected: true,
            durable_work_expected: false,
            current_answer_expected: true,
            semantic_queries: vec![concept.to_string()],
            required_capabilities: vec![concept.to_string()],
            rationale: Some("high confidence embedding fast path".to_string()),
            goals: vec![InboundTurnGoal {
                id: "g1".to_string(),
                intent_summary: "Handle the requested live or tool-mediated outcome".to_string(),
                capability_query: concept.to_string(),
                expected_outcome: "Requested outcome completed or answered".to_string(),
                durability: "none".to_string(),
                groundings: vec!["local_state".to_string()],
                side_effect: "none".to_string(),
                dependencies: Vec::new(),
            }],
            ..InboundRoutingSignal::default()
        },
        SecurityCategory::DurableWork | SecurityCategory::ManagedAppDelivery => {
            let (durability, intent_summary, expected_outcome) =
                if category == SecurityCategory::ManagedAppDelivery {
                    (
                        "deployment",
                        "Complete the requested app delivery outcome",
                        "Persistent app or browser-usable result is deployed or updated",
                    )
                } else {
                    durable_routing_shape(concept)
                };
            InboundRoutingSignal {
                should_execute: true,
                tool_use_expected: true,
                durable_work_expected: true,
                current_answer_expected: true,
                semantic_queries: vec![concept.to_string()],
                required_capabilities: vec![concept.to_string()],
                rationale: Some("high confidence embedding fast path".to_string()),
                goals: vec![InboundTurnGoal {
                    id: "g1".to_string(),
                    intent_summary: intent_summary.to_string(),
                    capability_query: concept.to_string(),
                    expected_outcome: expected_outcome.to_string(),
                    durability: durability.to_string(),
                    groundings: Vec::new(),
                    side_effect: "write".to_string(),
                    dependencies: Vec::new(),
                }],
                ..InboundRoutingSignal::default()
            }
        }
        SecurityCategory::SecurityBlock => InboundRoutingSignal::default(),
    }
}

fn block_decision(concept: &str) -> InboundClassificationDecision {
    InboundClassificationDecision {
        verdict: IntentVerdict::Block {
            message: "I can't help reveal, test, store, or route secrets or hidden instructions from chat. Use the secure credential flow or settings page for credentials, and keep private configuration out of messages.".to_string(),
            rule_id: format!("embedding-fast-block:{}", concept),
            severity: 80,
        },
        memory_capture: InboundMemoryCaptureSignal::default(),
        routing: InboundRoutingSignal::default(),
        direct_response: None,
        model_response: None,
    }
}

fn allow_decision(category: SecurityCategory, concept: &str) -> InboundClassificationDecision {
    InboundClassificationDecision {
        verdict: IntentVerdict::Allow,
        memory_capture: InboundMemoryCaptureSignal::default(),
        routing: routing_for_category(category, concept),
        direct_response: None,
        model_response: None,
    }
}

fn unchecked_route_decision(
    category: SecurityCategory,
    concept: &str,
) -> InboundClassificationDecision {
    InboundClassificationDecision {
        verdict: IntentVerdict::AllowWithUncheckedTag {
            reason: "embedding route was execution-shaped but below trusted fast-path threshold"
                .to_string(),
            intent_kinds: vec!["ambiguous".to_string()],
        },
        memory_capture: InboundMemoryCaptureSignal::default(),
        routing: routing_for_category(category, concept),
        direct_response: None,
        model_response: None,
    }
}

fn inbound_classifier_embedding_inputs(message: &str, context: &str) -> (String, String) {
    let message = message.trim();
    let _ = context;
    (message.to_string(), message.to_string())
}

fn memory_capture_signal_from_scores(
    scored: &[ScoredMemoryCaptureCanonical],
) -> InboundMemoryCaptureSignal {
    let best_capture = scored
        .iter()
        .filter(|item| item.should_capture)
        .max_by(|left, right| {
            left.score
                .partial_cmp(&right.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    let Some(best_capture) = best_capture else {
        return InboundMemoryCaptureSignal::default();
    };
    if best_capture.score < FAST_MEMORY_CAPTURE_CANDIDATE_MIN_SCORE {
        return InboundMemoryCaptureSignal::default();
    }
    let best_reject_score = scored
        .iter()
        .filter(|item| !item.should_capture)
        .map(|item| item.score)
        .fold(0.0_f32, f32::max);
    if best_reject_score >= FAST_MEMORY_CAPTURE_REJECT_MIN_SCORE
        && best_reject_score - best_capture.score >= FAST_MEMORY_CAPTURE_REJECT_MARGIN
    {
        return InboundMemoryCaptureSignal::default();
    }
    InboundMemoryCaptureSignal {
        should_capture: true,
        confidence: Some(best_capture.score.clamp(0.0, 1.0)),
        reason: Some(best_capture.concept.to_string()),
    }
}

fn embedding_fast_path_accepts(category: SecurityCategory, score: f32, margin: f32) -> bool {
    match category {
        SecurityCategory::SecurityBlock => score >= FAST_BLOCK_MIN_SCORE,
        SecurityCategory::DirectReply => {
            score >= FAST_ROUTING_MIN_SCORE && margin >= FAST_ROUTING_MARGIN
        }
        SecurityCategory::ToolUse => {
            score >= FAST_ROUTING_MIN_SCORE && margin >= FAST_ROUTING_MARGIN
        }
        SecurityCategory::DurableWork | SecurityCategory::ManagedAppDelivery => false,
    }
}

pub async fn classify_inbound_embedding_fast(
    embedding_client: &EmbeddingClient,
    normalized_message: &str,
    semantic_context: Option<&str>,
    data_dir: Option<&Path>,
) -> Result<Option<SecurityEmbeddingDecision>> {
    let message = normalized_message.trim();
    if message.is_empty() {
        return Ok(None);
    }
    let context = semantic_context.unwrap_or("").trim();
    let (routing_text_for_embedding, memory_text_for_embedding) =
        inbound_classifier_embedding_inputs(message, context);

    let mut canonicals = default_canonicals();
    canonicals.extend(load_overlay_canonicals(data_dir).await);
    let memory_canonicals = memory_capture_canonicals();
    let mut texts = Vec::with_capacity(canonicals.len() + memory_canonicals.len() + 2);
    texts.push(routing_text_for_embedding);
    texts.push(memory_text_for_embedding);
    texts.extend(
        canonicals
            .iter()
            .map(|canonical| canonical.text.to_string()),
    );
    texts.extend(
        memory_canonicals
            .iter()
            .map(|canonical| canonical.text.to_string()),
    );
    let embeddings = embedding_client.embed_texts(&texts).await?;
    let Some(message_embedding) = embeddings.first() else {
        return Ok(None);
    };
    let Some(memory_message_embedding) = embeddings.get(1) else {
        return Ok(None);
    };

    let mut scored = Vec::new();
    for (canonical, embedding) in canonicals.iter().zip(embeddings.iter().skip(2)) {
        let score =
            normalized_embedding_similarity(message_embedding.as_slice(), embedding.as_slice())
                .unwrap_or(0.0);
        scored.push(ScoredCanonical {
            category: canonical.category,
            concept: canonical.concept.clone(),
            score,
        });
    }
    let mut scored_memory = Vec::new();
    for (canonical, embedding) in memory_canonicals
        .iter()
        .zip(embeddings.iter().skip(2 + canonicals.len()))
    {
        let score = normalized_embedding_similarity(
            memory_message_embedding.as_slice(),
            embedding.as_slice(),
        )
        .unwrap_or(0.0);
        scored_memory.push(ScoredMemoryCaptureCanonical {
            should_capture: canonical.should_capture,
            concept: canonical.concept,
            score,
        });
    }
    let memory_capture = memory_capture_signal_from_scores(&scored_memory);

    let best = best_by_category(&scored);
    let Some(top) = best.values().max_by(|left, right| {
        left.score
            .partial_cmp(&right.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    }) else {
        return Ok(None);
    };
    let margin = category_margin(&best, top.category, top.score);

    let accepted = embedding_fast_path_accepts(top.category, top.score, margin);

    // When no category passes its trusted threshold, preserve only safe
    // execution-shaped routing hints. Direct-reply misses fall through to the
    // model router so chat responses stay model-generated without turning a
    // low-confidence embedding neighbor into trusted routing.
    if !accepted {
        return Ok(unaccepted_embedding_fallback(top, margin, memory_capture));
    }

    let mut decision = if top.category == SecurityCategory::SecurityBlock {
        block_decision(&top.concept)
    } else {
        allow_decision(top.category, &top.concept)
    };
    if matches!(decision.verdict, IntentVerdict::Allow) {
        decision.memory_capture = memory_capture;
    }
    Ok(Some(SecurityEmbeddingDecision {
        decision,
        category: top.category,
        score: top.score,
        margin,
        concept: top.concept.to_string(),
    }))
}

fn unaccepted_embedding_fallback(
    top: &ScoredCanonical,
    margin: f32,
    memory_capture: InboundMemoryCaptureSignal,
) -> Option<SecurityEmbeddingDecision> {
    if top.category == SecurityCategory::SecurityBlock && top.score >= FAST_BORDERLINE_BLOCK_SCORE {
        return None;
    }
    if matches!(
        top.category,
        SecurityCategory::DurableWork | SecurityCategory::ManagedAppDelivery
    ) && top.score >= FAST_UNCHECKED_ROUTE_MIN_SCORE
    {
        return None;
    }
    if matches!(top.category, SecurityCategory::ToolUse)
        && top.score >= FAST_UNCHECKED_ROUTE_MIN_SCORE
    {
        return Some(SecurityEmbeddingDecision {
            decision: unchecked_route_decision(top.category, &top.concept),
            category: top.category,
            score: top.score,
            margin,
            concept: top.concept.to_string(),
        });
    }
    if top.category == SecurityCategory::DirectReply {
        return None;
    }
    let _ = (margin, memory_capture);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_capture_embedding_input_uses_current_message_without_recent_context() {
        let (routing_input, memory_input) = inbound_classifier_embedding_inputs(
            "Stable profile fact.",
            "{\"recent_messages\":[{\"content\":\"Unrelated old task\"}]}",
        );

        assert!(routing_input.contains("Stable profile fact."));
        assert!(!routing_input.contains("Unrelated old task"));
        assert_eq!(memory_input, "Stable profile fact.");
        assert!(!memory_input.contains("Unrelated old task"));
    }

    #[test]
    fn routing_for_direct_reply_is_current_answer_only() {
        let routing = routing_for_category(SecurityCategory::DirectReply, "self_contained_answer");
        assert!(routing.current_answer_expected);
        assert!(!routing.should_execute);
        assert!(!routing.tool_use_expected);
        assert!(!routing.durable_work_expected);
    }

    #[test]
    fn routing_for_product_help_answer_uses_product_help_lookup() {
        let routing =
            routing_for_category(SecurityCategory::DirectReply, PRODUCT_HELP_ANSWER_CONCEPT);
        assert!(routing.current_answer_expected);
        assert!(routing.should_execute);
        assert!(routing.tool_use_expected);
        assert!(!routing.durable_work_expected);
        assert!(routing.product_help_expected);
        assert!(routing.has_transient_read_only_lookup());
        assert_eq!(
            routing.goals[0].groundings,
            vec!["product_help".to_string()]
        );
    }

    #[test]
    fn routing_for_managed_app_delivery_is_deployment() {
        let routing =
            routing_for_category(SecurityCategory::ManagedAppDelivery, "managed_app_delivery");
        assert!(routing.should_execute);
        assert!(routing.tool_use_expected);
        assert!(routing.durable_work_expected);
        assert_eq!(routing.goals[0].durability, "deployment");
    }

    #[test]
    fn durable_canonicals_preserve_semantic_delivery_shape() {
        let scheduled = routing_for_category(SecurityCategory::DurableWork, SCHEDULED_TASK_CONCEPT);
        assert_eq!(scheduled.goals[0].durability, "scheduled_time");
        assert_eq!(
            scheduled.semantic_turn_plan().goals[0].delivery_kind,
            "scheduled_task"
        );

        let watcher = routing_for_category(SecurityCategory::DurableWork, WATCHER_MONITOR_CONCEPT);
        assert_eq!(watcher.goals[0].durability, "recurring_monitor");
        assert_eq!(
            watcher.semantic_turn_plan().goals[0].delivery_kind,
            "watcher_monitor"
        );
    }

    #[test]
    fn durable_shapes_never_use_trusted_embedding_fast_path() {
        for category in [
            SecurityCategory::DurableWork,
            SecurityCategory::ManagedAppDelivery,
        ] {
            assert!(!embedding_fast_path_accepts(category, 0.99, 0.99));
        }
        assert!(embedding_fast_path_accepts(
            SecurityCategory::DirectReply,
            FAST_ROUTING_MIN_SCORE,
            FAST_ROUTING_MARGIN
        ));
        assert!(embedding_fast_path_accepts(
            SecurityCategory::ToolUse,
            FAST_ROUTING_MIN_SCORE,
            FAST_ROUTING_MARGIN
        ));
    }

    #[test]
    fn memory_capture_can_be_true_for_direct_reply_routing() {
        let signal = memory_capture_signal_from_scores(&[
            ScoredMemoryCaptureCanonical {
                should_capture: true,
                concept: "durable_user_identity_profile",
                score: 0.82,
            },
            ScoredMemoryCaptureCanonical {
                should_capture: false,
                concept: "transient_social_or_acknowledgement",
                score: 0.55,
            },
        ]);
        assert!(signal.should_capture);
        assert_eq!(
            signal.reason.as_deref(),
            Some("durable_user_identity_profile")
        );
    }

    #[test]
    fn memory_capture_rejects_transient_top_score() {
        let signal = memory_capture_signal_from_scores(&[
            ScoredMemoryCaptureCanonical {
                should_capture: true,
                concept: "artifact_outcome_feedback",
                score: 0.60,
            },
            ScoredMemoryCaptureCanonical {
                should_capture: false,
                concept: "transient_social_or_acknowledgement",
                score: 0.78,
            },
        ]);
        assert!(!signal.should_capture);
    }

    #[test]
    fn unaccepted_ambiguous_direct_reply_escalates_to_structured_classifier() {
        let fallback = unaccepted_embedding_fallback(
            &ScoredCanonical {
                category: SecurityCategory::DirectReply,
                concept: "low_confidence_direct".to_string(),
                score: 0.40,
            },
            0.01,
            InboundMemoryCaptureSignal {
                should_capture: true,
                confidence: Some(0.81),
                reason: Some("durable_user_identity_profile".to_string()),
            },
        );

        assert!(fallback.is_none());
    }

    #[test]
    fn unaccepted_direct_reply_escalates_to_structured_classifier_even_when_moderate() {
        let fallback = unaccepted_embedding_fallback(
            &ScoredCanonical {
                category: SecurityCategory::DirectReply,
                concept: "moderate_confidence_direct".to_string(),
                score: 0.71,
            },
            0.07,
            InboundMemoryCaptureSignal {
                should_capture: true,
                confidence: Some(0.81),
                reason: Some("durable_user_identity_profile".to_string()),
            },
        );

        assert!(
            fallback.is_none(),
            "untrusted direct-reply embeddings must not bypass the structured router"
        );
    }

    #[test]
    fn unaccepted_execution_shape_routes_without_direct_reply_trust() {
        let fallback = unaccepted_embedding_fallback(
            &ScoredCanonical {
                category: SecurityCategory::ToolUse,
                concept: "live_state_or_external_lookup".to_string(),
                score: FAST_UNCHECKED_ROUTE_MIN_SCORE,
            },
            0.0,
            InboundMemoryCaptureSignal::default(),
        )
        .expect("execution-shaped traffic should keep its route shape");

        assert_eq!(fallback.category, SecurityCategory::ToolUse);
        assert!(matches!(
            fallback.decision.verdict,
            IntentVerdict::AllowWithUncheckedTag { .. }
        ));
        assert!(fallback.decision.routing.should_execute);
        assert!(fallback.decision.routing.tool_use_expected);
        assert!(fallback.decision.routing.current_answer_expected);
    }

    #[test]
    fn unaccepted_durable_shape_escalates_to_structured_classifier() {
        for category in [
            SecurityCategory::DurableWork,
            SecurityCategory::ManagedAppDelivery,
        ] {
            let fallback = unaccepted_embedding_fallback(
                &ScoredCanonical {
                    category,
                    concept: "durable_or_delivery_neighbor".to_string(),
                    score: FAST_UNCHECKED_ROUTE_MIN_SCORE,
                },
                0.0,
                InboundMemoryCaptureSignal::default(),
            );

            assert!(
                fallback.is_none(),
                "weak durable embeddings must not force write/deploy routing"
            );
        }
    }

    #[test]
    fn unaccepted_borderline_security_block_still_escalates() {
        let fallback = unaccepted_embedding_fallback(
            &ScoredCanonical {
                category: SecurityCategory::SecurityBlock,
                concept: "credential_exfiltration".to_string(),
                score: FAST_BORDERLINE_BLOCK_SCORE,
            },
            0.0,
            InboundMemoryCaptureSignal::default(),
        );

        assert!(fallback.is_none());
    }
}
