use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::Path;

use anyhow::Result;
use sea_orm::entity::prelude::PgVector;

use crate::core::knowledge::document_search::normalized_embedding_similarity;
use crate::core::EmbeddingClient;

use super::intent_classifier::{
    InboundAdvisorySignal, InboundClassificationDecision, InboundMemoryCaptureSignal, IntentVerdict,
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
const FAST_CONVERSATIONAL_MIN_SCORE: f32 = 0.70;
const FAST_CONVERSATIONAL_MARGIN: f32 = 0.06;
const FAST_UNCHECKED_CLASSIFICATION_MIN_SCORE: f32 = 0.58;
const FAST_CLASSIFIER_CONTEXT_MAX_CHARS: usize = 2400;
const PRODUCT_IDENTITY_ANSWER_CONCEPT: &str = "product_identity_answer";
const AGENTARK_CAPABILITIES_ANSWER_CONCEPT: &str = "agentark_capabilities_answer";
const SCHEDULED_TASK_CONCEPT: &str = "scheduled_task";
const WATCHER_MONITOR_CONCEPT: &str = "watcher_monitor";
const INTEGRATION_SETUP_CONCEPT: &str = "integration_setup";
const PERSISTENT_ARTIFACT_CONCEPT: &str = "persistent_artifact";
const CANONICAL_EMBEDDING_CACHE_MAX_ENTRIES: usize = 8;

static SECURITY_CANONICAL_EMBEDDING_CACHE: once_cell::sync::Lazy<
    tokio::sync::Mutex<HashMap<String, CachedSecurityCanonicalEmbeddings>>,
> = once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SecurityCategory {
    Conversational,
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
    pub advisory: InboundAdvisorySignal,
}

#[derive(Debug, Clone)]
struct ScoredCanonical {
    category: SecurityCategory,
    concept: String,
    score: f32,
    advisory: InboundAdvisorySignal,
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
struct CachedSecurityCanonicalEmbeddings {
    canonical_embeddings: Vec<PgVector>,
    memory_embeddings: Vec<PgVector>,
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
        MemoryCaptureCanonical {
            should_capture: false,
            concept: "operational_schedule_or_notification_request",
            text: "The message asks the assistant to create, schedule, send, route, or manage a specific reminder, meeting notification, alert, automation, watcher, or task. The people, channel, time, trigger, and event details belong to that work item, not reusable user memory.",
        },
    ]
}

fn default_canonicals() -> Vec<SecurityCanonical> {
    fn canonical(
        category: SecurityCategory,
        concept: &'static str,
        text: &'static str,
    ) -> SecurityCanonical {
        canonical_with_advisory(
            category,
            concept,
            text,
            default_advisory_for_category(category, concept),
        )
    }

    fn canonical_with_advisory(
        category: SecurityCategory,
        concept: &'static str,
        text: &'static str,
        advisory: InboundAdvisorySignal,
    ) -> SecurityCanonical {
        SecurityCanonical {
            category,
            concept: concept.to_string(),
            text: text.to_string(),
            advisory,
        }
    }

    fn default_advisory_for_category(
        category: SecurityCategory,
        concept: &'static str,
    ) -> InboundAdvisorySignal {
        let mut advisory = InboundAdvisorySignal::default();
        advisory
            .semantic_queries
            .push(format!("semantic_concept:{concept}"));
        advisory.rationale = Some("high-confidence embedding route".to_string());
        match category {
            SecurityCategory::Conversational => {
                advisory.current_answer_expected = true;
            }
            SecurityCategory::ToolUse => {
                advisory.should_execute = true;
                advisory.tool_use_expected = true;
            }
            SecurityCategory::DurableWork | SecurityCategory::ManagedAppDelivery => {
                advisory.should_execute = true;
                advisory.tool_use_expected = true;
                advisory.durable_work_expected = true;
            }
            SecurityCategory::SecurityBlock => {}
        }
        advisory
    }

    vec![
        canonical(
            SecurityCategory::Conversational,
            "self_contained_answer",
            "The user wants a current conversational answer, explanation, acknowledgement, clarification, or answer that can be given directly from the conversation context without live tools, external retrieval, durable side effects, saved artifacts, deployments, schedules, integrations, or state inspection.",
        ),
        canonical(
            SecurityCategory::Conversational,
            PRODUCT_IDENTITY_ANSWER_CONCEPT,
            "The user asks for the assistant or running product identity, name, who it is, or what it should call itself, and the answer should come from the trusted product identity already supplied by the system.",
        ),
        canonical_with_advisory(
            SecurityCategory::Conversational,
            AGENTARK_CAPABILITIES_ANSWER_CONCEPT,
            "The user asks what AgentArk can do, how an AgentArk capability works, or where an AgentArk feature is configured; the answer should be grounded in the live AgentArk capability registry with curated manual context only as supplemental explanation, not in the assistant's trusted product identity and not in live logs or external web.",
            InboundAdvisorySignal {
                current_answer_expected: true,
                agentark_capabilities_expected: true,
                semantic_queries: vec![format!(
                    "semantic_concept:{AGENTARK_CAPABILITIES_ANSWER_CONCEPT}"
                )],
                required_capabilities: vec![
                    "agentark capability registry and manual context".to_string(),
                ],
                rationale: Some("high-confidence embedding route".to_string()),
                ..Default::default()
            },
        ),
        canonical(
            SecurityCategory::Conversational,
            "previous_user_turn_recall",
            "The user asks to recall, quote, restate, identify, or answer from the immediately previous user message or question in the visible conversation history.",
        ),
        canonical(
            SecurityCategory::Conversational,
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
            "The user wants a durable background task whose trigger is a known clock time, date, deadline, interval, calendar-like recurrence, reminder time, or timed follow-up. This is not for external-state change detection or polling a source until a condition appears.",
        ),
        canonical(
            SecurityCategory::DurableWork,
            WATCHER_MONITOR_CONCEPT,
            "The user wants durable monitoring, watching, polling, tracking, or alerting based on a changing condition in an external, local, message, feed, page, camera, news, pricing, app, file, document, workspace, or system source. The trigger is new, changed, missing, threshold-crossing, or otherwise condition-based state, and notification is the delivery route after the condition is met.",
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
        "conversational" => Some(SecurityCategory::Conversational),
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
                advisory: InboundAdvisorySignal {
                    current_answer_expected: matches!(category, SecurityCategory::Conversational),
                    should_execute: matches!(
                        category,
                        SecurityCategory::ToolUse
                            | SecurityCategory::DurableWork
                            | SecurityCategory::ManagedAppDelivery
                    ),
                    tool_use_expected: matches!(
                        category,
                        SecurityCategory::ToolUse
                            | SecurityCategory::DurableWork
                            | SecurityCategory::ManagedAppDelivery
                    ),
                    durable_work_expected: matches!(
                        category,
                        SecurityCategory::DurableWork | SecurityCategory::ManagedAppDelivery
                    ),
                    semantic_queries: vec![format!("overlay_semantic_concept:{concept}")],
                    rationale: Some("overlay embedding route".to_string()),
                    ..Default::default()
                },
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

fn block_decision(concept: &str) -> InboundClassificationDecision {
    InboundClassificationDecision {
        verdict: IntentVerdict::Block {
            message: "I can't help reveal, test, store, or route secrets or hidden instructions from chat. Use the secure credential flow or settings page for credentials, and keep private configuration out of messages.".to_string(),
            rule_id: format!("embedding-fast-block:{}", concept),
            severity: 80,
        },
        memory_capture: InboundMemoryCaptureSignal::default(),
        advisory: InboundAdvisorySignal::default(),
        model_response: None,
    }
}

fn allow_decision(
    category: SecurityCategory,
    concept: &str,
    advisory: &InboundAdvisorySignal,
) -> InboundClassificationDecision {
    let _ = (category, concept);
    // Embedding classifier does not choose concrete tools. It only forwards
    // typed advisory so later planning can skip redundant review when the
    // semantic route is already high-confidence.
    InboundClassificationDecision {
        verdict: IntentVerdict::Allow,
        memory_capture: InboundMemoryCaptureSignal::default(),
        advisory: advisory.clone(),
        model_response: None,
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    out
}

fn inbound_classifier_embedding_inputs(message: &str, context: &str) -> (String, String) {
    let message = message.trim();
    let context = context.trim();
    let classification_input = if context.is_empty() {
        message.to_string()
    } else {
        format!(
            "Current user message:\n{}\n\nRecent semantic context for continuity:\n{}",
            message,
            truncate_chars(context, FAST_CLASSIFIER_CONTEXT_MAX_CHARS)
        )
    };
    (classification_input, message.to_string())
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

fn embedding_security_quick_accepts(category: SecurityCategory, score: f32, margin: f32) -> bool {
    match category {
        SecurityCategory::SecurityBlock => score >= FAST_BLOCK_MIN_SCORE,
        SecurityCategory::Conversational => {
            score >= FAST_CONVERSATIONAL_MIN_SCORE && margin >= FAST_CONVERSATIONAL_MARGIN
        }
        SecurityCategory::ToolUse => false,
        SecurityCategory::DurableWork | SecurityCategory::ManagedAppDelivery => false,
    }
}

fn canonical_embedding_cache_key(
    embedding_client: &EmbeddingClient,
    canonicals: &[SecurityCanonical],
    memory_canonicals: &[MemoryCaptureCanonical],
) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    embedding_client.describe_backend().hash(&mut hasher);
    for canonical in canonicals {
        canonical.category.hash(&mut hasher);
        canonical.concept.hash(&mut hasher);
        canonical.text.hash(&mut hasher);
    }
    for canonical in memory_canonicals {
        canonical.should_capture.hash(&mut hasher);
        canonical.concept.hash(&mut hasher);
        canonical.text.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

async fn load_or_embed_canonical_embeddings(
    embedding_client: &EmbeddingClient,
    canonicals: &[SecurityCanonical],
    memory_canonicals: &[MemoryCaptureCanonical],
) -> Result<CachedSecurityCanonicalEmbeddings> {
    let key = canonical_embedding_cache_key(embedding_client, canonicals, memory_canonicals);
    if let Some(cached) = SECURITY_CANONICAL_EMBEDDING_CACHE
        .lock()
        .await
        .get(&key)
        .cloned()
    {
        return Ok(cached);
    }

    let mut texts = Vec::with_capacity(canonicals.len() + memory_canonicals.len());
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
    let cached = CachedSecurityCanonicalEmbeddings {
        canonical_embeddings: embeddings
            .iter()
            .take(canonicals.len())
            .cloned()
            .collect::<Vec<_>>(),
        memory_embeddings: embeddings
            .iter()
            .skip(canonicals.len())
            .take(memory_canonicals.len())
            .cloned()
            .collect::<Vec<_>>(),
    };
    if cached.canonical_embeddings.len() != canonicals.len()
        || cached.memory_embeddings.len() != memory_canonicals.len()
    {
        return Ok(cached);
    }

    let mut cache = SECURITY_CANONICAL_EMBEDDING_CACHE.lock().await;
    if cache.len() >= CANONICAL_EMBEDDING_CACHE_MAX_ENTRIES {
        if let Some(evict_key) = cache.keys().next().cloned() {
            cache.remove(&evict_key);
        }
    }
    cache.insert(key, cached.clone());
    Ok(cached)
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
    let (classification_text_for_embedding, memory_text_for_embedding) =
        inbound_classifier_embedding_inputs(message, context);

    let mut canonicals = default_canonicals();
    canonicals.extend(load_overlay_canonicals(data_dir).await);
    let memory_canonicals = memory_capture_canonicals();
    let turn_texts = vec![classification_text_for_embedding, memory_text_for_embedding];
    let (turn_embeddings, cached_embeddings) = tokio::try_join!(
        embedding_client.embed_texts(&turn_texts),
        load_or_embed_canonical_embeddings(embedding_client, &canonicals, &memory_canonicals),
    )?;
    let Some(message_embedding) = turn_embeddings.first() else {
        return Ok(None);
    };
    let Some(memory_message_embedding) = turn_embeddings.get(1) else {
        return Ok(None);
    };

    let mut scored = Vec::new();
    for (canonical, embedding) in canonicals
        .iter()
        .zip(cached_embeddings.canonical_embeddings.iter())
    {
        let score =
            normalized_embedding_similarity(message_embedding.as_slice(), embedding.as_slice())
                .unwrap_or(0.0);
        scored.push(ScoredCanonical {
            category: canonical.category,
            concept: canonical.concept.clone(),
            score,
            advisory: canonical.advisory.clone(),
        });
    }
    let mut scored_memory = Vec::new();
    for (canonical, embedding) in memory_canonicals
        .iter()
        .zip(cached_embeddings.memory_embeddings.iter())
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

    let accepted = embedding_security_quick_accepts(top.category, top.score, margin);

    let context_present = !context.is_empty();
    // Context-bearing turns may depend on earlier objects, sources, or
    // subjects. Let the structured classifier inspect that context instead of
    // relying on an embedding-only decision. Security blocks remain eligible
    // for quick handling.
    if context_present && !matches!(top.category, SecurityCategory::SecurityBlock) {
        return Ok(None);
    }

    // When no category passes its trusted threshold, keep only security
    // decisions in this layer. Lookup, durable work, and tool selection are
    // handled by the main model tool loop.
    if !accepted {
        return Ok(unaccepted_embedding_fallback(
            top,
            margin,
            memory_capture,
            context_present,
        ));
    }

    let mut decision = if top.category == SecurityCategory::SecurityBlock {
        block_decision(&top.concept)
    } else {
        allow_decision(top.category, &top.concept, &top.advisory)
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
    context_present: bool,
) -> Option<SecurityEmbeddingDecision> {
    if context_present && !matches!(top.category, SecurityCategory::SecurityBlock) {
        return None;
    }
    if top.category == SecurityCategory::SecurityBlock && top.score >= FAST_BORDERLINE_BLOCK_SCORE {
        return Some(SecurityEmbeddingDecision {
            decision: block_decision(&top.concept),
            category: top.category,
            score: top.score,
            margin,
            concept: top.concept.to_string(),
        });
    }
    if matches!(
        top.category,
        SecurityCategory::DurableWork | SecurityCategory::ManagedAppDelivery
    ) && top.score >= FAST_UNCHECKED_CLASSIFICATION_MIN_SCORE
    {
        return None;
    }
    if matches!(top.category, SecurityCategory::ToolUse)
        && top.score >= FAST_UNCHECKED_CLASSIFICATION_MIN_SCORE
    {
        let _ = memory_capture;
        return None;
    }
    if top.category == SecurityCategory::Conversational
        && top.score >= FAST_UNCHECKED_CLASSIFICATION_MIN_SCORE
    {
        return None;
    }
    let _ = (margin, memory_capture);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_canonicals_include_operational_notification_reject_neighbor() {
        let canonicals = memory_capture_canonicals();

        assert!(canonicals.iter().any(|canonical| {
            !canonical.should_capture
                && canonical.concept == "operational_schedule_or_notification_request"
                && canonical.text.contains("work item")
        }));
    }

    #[test]
    fn memory_capture_scores_reject_operational_schedule_payloads() {
        let signal = memory_capture_signal_from_scores(&[
            ScoredMemoryCaptureCanonical {
                should_capture: true,
                concept: "reusable_project_context",
                score: 0.78,
            },
            ScoredMemoryCaptureCanonical {
                should_capture: false,
                concept: "operational_schedule_or_notification_request",
                score: 0.86,
            },
        ]);

        assert!(!signal.should_capture);
    }
}
