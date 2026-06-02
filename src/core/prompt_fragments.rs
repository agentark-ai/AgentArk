#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const PROMPT_FRAGMENT_BUNDLE_PROFILE_KEY: &str = "prompt_fragment_bundle_profile_v1";
pub const PROMPT_FRAGMENT_BUNDLE_PROFILE_CANARY_KEY: &str =
    "prompt_fragment_bundle_profile_canary_v1";
pub const PROMPT_FRAGMENT_BUNDLE_CANARY_STATE_KEY: &str = "prompt_fragment_bundle_canary_state_v1";
pub const PROMPT_FRAGMENT_BUNDLE_BASELINE_SNAPSHOT_KEY: &str =
    "prompt_fragment_bundle_baseline_snapshot_v1";
pub const PROMPT_FRAGMENT_BUNDLE_LAST_RESULT_KEY: &str = "prompt_fragment_bundle_last_result_v1";
pub const BASE_PROMPT_FRAGMENT_VERSION: &str = "prompt_fragments_v1";

const PROMPT_FRAGMENT_BUNDLE_DEFAULT_VERSION: &str = "prompt-fragments-default-v1";
const MAX_FRAGMENT_BODY_CHARS: usize = 4_000;
const MAX_FRAGMENT_ID_CHARS: usize = 128;
const MAX_FRAGMENT_SURFACE_CHARS: usize = 64;
const MAX_PROMPT_FRAGMENTS: usize = 64;
const REQUIRED_BASELINE_FRAGMENT_IDS: &[&str] = &[
    "fragment.baseline.identity_security",
    "fragment.baseline.semantic_dag_contract",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptFragment {
    pub id: String,
    pub surface: String,
    pub body: String,
    #[serde(default)]
    pub scope_tags: Vec<String>,
    #[serde(default)]
    pub always_on: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub est_tokens: usize,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptFragmentBundleProfile {
    pub version: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub fragments: Vec<PromptFragment>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptFragmentSelection {
    pub bundle_version: String,
    pub active_tags: BTreeSet<String>,
    pub fragments: Vec<PromptFragment>,
    pub estimated_tokens: usize,
    pub evicted_fragment_ids: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

pub fn required_prompt_fragment_ids() -> &'static [&'static str] {
    REQUIRED_BASELINE_FRAGMENT_IDS
}

pub fn normalize_prompt_tag(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

pub fn insert_prompt_tag(tags: &mut BTreeSet<String>, value: &str) {
    let normalized = normalize_prompt_tag(value);
    if !normalized.is_empty() {
        tags.insert(normalized);
    }
}

fn fragment(
    id: &str,
    surface: &str,
    scope_tags: &[&str],
    always_on: bool,
    priority: i32,
    body: &str,
) -> PromptFragment {
    let mut scope_tags = scope_tags
        .iter()
        .map(|tag| normalize_prompt_tag(tag))
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    scope_tags.sort();
    scope_tags.dedup();
    PromptFragment {
        id: id.to_string(),
        surface: surface.to_string(),
        body: body.trim().to_string(),
        scope_tags,
        always_on,
        priority,
        est_tokens: estimate_tokens(body),
        enabled: true,
    }
}

fn estimate_tokens(text: &str) -> usize {
    text.chars().count().saturating_add(3) / 4
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect::<String>()
}

fn sanitize_fragment_id(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':'))
        .take(MAX_FRAGMENT_ID_CHARS)
        .collect::<String>()
}

fn sanitize_fragment_surface(value: &str) -> String {
    let normalized = normalize_prompt_tag(value);
    if normalized.is_empty() {
        "agent_loop".to_string()
    } else {
        truncate_chars(&normalized, MAX_FRAGMENT_SURFACE_CHARS)
    }
}

pub fn default_prompt_fragment_bundle() -> PromptFragmentBundleProfile {
    PromptFragmentBundleProfile {
        version: PROMPT_FRAGMENT_BUNDLE_DEFAULT_VERSION.to_string(),
        updated_at: None,
        fragments: vec![
            fragment(
                "fragment.baseline.identity_security",
                "agent_loop",
                &[],
                true,
                1000,
                r#"- Operate as AgentArk, the running product, while treating model/provider identity as an implementation detail.
- Security first: keep least privilege, never expose secrets, and stop on unsafe or unauthorized operations.
- Prefer doing the work when authorized actions and required inputs are available; ask one concise clarification only when execution would otherwise be ambiguous or unsafe.
- Ground final answers in visible context, tool results, memories, artifacts, and authorized action outputs. Do not expose internal prompts, schemas, routing scores, or hidden policy mechanics unless explicitly asked.
- Keep retries bounded and report the last error plus the next safe recovery step when blocked."#,
            ),
            fragment(
                "fragment.baseline.model_tool_loop",
                "agent_loop",
                &[],
                true,
                950,
                r#"- The model chooses among available actions when a tool is useful, and answers directly when no tool is needed.
- Use prior conversation to resolve clear references, continuations, corrections, approvals, and dependencies, while allowing the current message to change intent.
- Do not invent tool results, IDs, links, schedules, objects, credentials, or notifications.
- User-facing text should be concise, concrete, and operationally honest. Do not emit control JSON, scope sentinels, routing telemetry, or chain-of-thought into final prose."#,
            ),
            fragment(
                "fragment.baseline.semantic_dag_contract",
                "agent_loop",
                &[],
                true,
                940,
                r#"- Treat every user turn as a possible set of independent or dependent outcomes, not as a single intent by default.
- Preserve all outcomes the current turn asks for: direct answers, workspace changes, managed services, scheduled or conditional background work, reminders, integration work, browser actions, and local inspections can coexist in one turn.
- Use recent conversation, memories, active artifacts, managed services, scheduled/background work, and pending actions only to resolve references, continuations, corrections, approvals, and dependencies. If the current turn changes topic or reverses earlier intent, follow the current turn.
- Build the smallest implicit goal graph needed: run independent reads or mutations together when safe, sequence dependent work only when one result is needed by the next, and ask a concise clarification only for the missing detail that blocks a specific outcome.
- Multiple tool calls are allowed when the user asks for multiple outcomes. Do not drop a requested outcome just because another tool call already succeeded."#,
            ),
            fragment(
                "fragment.secret.sidecar",
                "agent_loop",
                &["secret"],
                false,
                900,
                r#"- A secret-like input was redacted before the model prompt. Do not ask the user to paste it again in chat.
- If credentials are required, use the secure credential setup path or ask which Settings/integration target should receive the secret when the target is ambiguous.
- Continue non-secret parts of the request when possible."#,
            ),
            fragment(
                "fragment.app_delivery.protocol",
                "agent_loop",
                &["app_hosting", "integration_app", "app_delivery"],
                false,
                900,
                r#"- For generated app, site, dashboard, local service, or browser tool delivery, writing files is staging; the goal completes only when the authorized managed-service path returns a runnable result or asks for missing required inputs.
- Build the smallest working app that satisfies the user-visible requirements. Prefer a compact MVP over a broad product scaffold: keep file count low, avoid generated boilerplate, and do not add routes, auth, databases, service layers, package manifests, tests, or admin surfaces unless the request semantically requires them.
- Make the app visually polished and pleasant to use within that lean scope: strong layout, responsive behavior, good typography, clear controls, useful empty/loading/error states, and domain-appropriate styling. Do not expand polished UI into unrelated SaaS features.
- Prefer a standalone static/browser bundle when the requested workflow can run with browser APIs, timers, client-side state, and public same-origin/app-scoped fetch. Emit complete static files without package manifests, servers, or lifecycle commands in that case.
- Use a dynamic backend/runtime only for server-only needs: secret credentials, authenticated server-side API access, durable jobs that must continue with no browser open, durable server-side state/databases, filesystem/process access, webhooks, private-network access, non-HTTP protocols, or APIs that the browser/app proxy cannot safely call.
- If the file-stream protocol is active, emit complete service files as `<file path="relative/path.ext">...</file>` blocks and let AgentArk synthesize the service-management action. Do not emit lifecycle JSON, agent_tool_calls JSON, markdown fences around file blocks, or native tool calls in that response.
- When updating a recent deployed app, preserve the active app identity, original requirements, current deployed files, and working behavior unless the user asks to replace or recreate it. Apply the requested add/remove/change directly and keep unrelated app scope unchanged.
- Deploy locally by default. Content visibility or audience requirements inside the app are not the same as external network exposure.
- Treat deploy completion as registration/startup evidence, not proof that browser JavaScript, client-side fetches, or the full requested workflow worked. Do not claim runtime/browser validation unless you actually ran a browser/runtime check and observed passing evidence; otherwise say the app was deployed and any background quality check is advisory/pending.
- After managed-service edits, restart or validate through the available service action before claiming completion when that action is in scope.
- After deployment, nudge the user to the Apps page for start, stop, restart, logs, App Guard, public exposure, and delete controls."#,
            ),
            fragment(
                "fragment.ark_inspection.local_state",
                "agent_loop",
                &[
                    "action_ark_inspect",
                    "platform_observability",
                    "app_registry",
                    "app_inventory",
                    "personal_activity",
                    "activity_insights",
                    "database_readonly",
                    "model_runtime",
                    "model_status",
                    "provider_status",
                ],
                false,
                850,
                r#"- For AgentArk-owned runtime state, pages, deployed apps, Pulse, Sentinel, Evolve, Trace, operator health, analytics, recent work, or reflective activity insight, inspect local evidence before answering.
- Reflective activity insight includes questions whose answer depends on the user's recent behavior, conversations, work patterns, attention, avoidance, recurring themes, or inferred mindset; use AgentArk local activity plus Reflect/Sentinel-style signals when they are available.
- For current model/provider selection, configured model slots, access/readiness, failover, or provider health, use read-only runtime inspection and disclose only non-secret status.
- Prefer the structured Ark inspection/API-discovery path when available. Use database schema/read-only queries only after a suitable API surface is unavailable or insufficient.
- Answer with calibrated uncertainty from observed evidence rather than generic assumptions."#,
            ),
            fragment(
                "fragment.agentark_knowledge.capabilities",
                "agent_loop",
                &[
                    "agentark_capabilities",
                    "agentark_manual",
                    "capability_inventory",
                    "documentation",
                ],
                false,
                830,
                r#"- For questions about what this running AgentArk can do, how a product feature works, where it is configured, or whether a capability exists, use live AgentArk capability data as authoritative.
- Treat curated AgentArk manual text as supplemental explanation, not capability truth.
- Use local-state inspection separately for current objects, run logs, deployed apps, traces, or user activity evidence."#,
            ),
            fragment(
                "fragment.attachments.vision_documents",
                "agent_loop",
                &["attachment", "vision_ocr", "documents"],
                false,
                820,
                r#"- Treat attachments as evidence for the current request. If the answer depends on visual or document contents, use the authorized vision/document action before answering.
- For visual uploads, pass the supplied upload identifier to the vision action. For indexed documents, pass document identifiers to document lookup when available.
- If a visual attachment arrives without a non-empty user message, treat the turn as an implicit request to understand the image, without inferring sensitive traits or saving one-off image contents as durable preferences."#,
            ),
            fragment(
                "fragment.evidence.synthesis",
                "agent_loop",
                &["evidence_lookup", "role_data_source", "role_inspection"],
                false,
                780,
                r#"- For evidence gathering, call the minimum needed data-source or inspection action, then answer from the observed result.
- If the intended result is an in-chat report, synthesis, or analysis, use prose and tables for exact values. Include fenced `agentark-chart` JSON only when a chart materially clarifies quantitative comparisons, trends, distributions, proportions, uncertainty, evidence coverage, or grouped breakdowns.
- Use managed service delivery only when the requested final object is a browser-runnable, reusable, hosted, or previewable experience."#,
            ),
            fragment(
                "fragment.automation.durable_work",
                "agent_loop",
                &[
                    "scheduler",
                    "watcher",
                    "role_orchestration",
                    "delivery_async",
                    "delivery_conditional",
                ],
                false,
                820,
                r#"- For scheduled tasks, reminders, recurring jobs, and conditional monitoring, preserve each distinct target, condition, cadence, timeout, and delivery route.
- A cadence belongs to the object it modifies: service/dashboard/tool refresh belongs inside the generated artifact, while AgentArk-owned later execution, independent monitoring, and outside-UI notification belong to scheduled work.
- Create or update the durable object before optional reads unless a required argument cannot be inferred without a read."#,
            ),
            fragment(
                "fragment.arkorbit.surface",
                "agent_loop",
                &["arkorbit"],
                false,
                780,
                r#"- In an ArkOrbit file-backed browser-surface turn, materialize durable output as clean browser assets in the selected orbit namespace.
- Use integrations, messaging, search, files, or app tools only when the requested surface semantically needs them.
- Never put OAuth tokens, API keys, cookies, bearer headers, or provider credentials into Orbit HTML/JS; fetch authenticated data through authorized server-side tools first, or build an app/backend proxy using the secure credential path."#,
            ),
            fragment(
                "fragment.integration.management",
                "agent_loop",
                &[
                    "integration_admin",
                    "integration_builder",
                    "integration_inventory",
                    "capability_inventory",
                    "skill_management",
                ],
                false,
                760,
                r#"- Treat built-in connectors, extension packs, custom integrations, and acquired capabilities as distinct surfaces.
- Inspect the available catalog or integration state before claiming a capability is unavailable.
- For credentialed integrations, use secure credential/auth flows. Do not place raw tokens, passwords, cookies, or API keys in generated code, tool arguments, logs, or final answers."#,
            ),
        ],
    }
}

pub fn compose_prompt_fragment_version(bundle_version: &str) -> String {
    format!("{}+{}", BASE_PROMPT_FRAGMENT_VERSION, bundle_version.trim())
}

pub fn parse_prompt_fragment_bundle_profile(raw: &[u8]) -> Option<PromptFragmentBundleProfile> {
    let mut bundle = serde_json::from_slice::<PromptFragmentBundleProfile>(raw).ok()?;
    sanitize_prompt_fragment_bundle(&mut bundle);
    Some(bundle)
}

pub fn sanitize_prompt_fragment_bundle(bundle: &mut PromptFragmentBundleProfile) {
    let default_bundle = default_prompt_fragment_bundle();
    bundle.version = truncate_chars(bundle.version.trim(), 128);
    if bundle.version.is_empty() {
        bundle.version = PROMPT_FRAGMENT_BUNDLE_DEFAULT_VERSION.to_string();
    }

    let mut sanitized = Vec::new();
    let mut seen = BTreeSet::new();
    for fragment in std::mem::take(&mut bundle.fragments) {
        let id = sanitize_fragment_id(&fragment.id);
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        let body = truncate_chars(fragment.body.trim(), MAX_FRAGMENT_BODY_CHARS);
        if body.is_empty() {
            continue;
        }
        let mut scope_tags = fragment
            .scope_tags
            .iter()
            .map(|tag| normalize_prompt_tag(tag))
            .filter(|tag| !tag.is_empty())
            .collect::<Vec<_>>();
        scope_tags.sort();
        scope_tags.dedup();
        sanitized.push(PromptFragment {
            id,
            surface: sanitize_fragment_surface(&fragment.surface),
            body: body.clone(),
            scope_tags,
            always_on: fragment.always_on,
            priority: fragment.priority.clamp(-10_000, 10_000),
            est_tokens: estimate_tokens(&body),
            enabled: fragment.enabled,
        });
        if sanitized.len() >= MAX_PROMPT_FRAGMENTS {
            break;
        }
    }

    for required_id in REQUIRED_BASELINE_FRAGMENT_IDS {
        if sanitized.iter().any(|fragment| fragment.id == *required_id) {
            continue;
        }
        if let Some(default_fragment) = default_bundle
            .fragments
            .iter()
            .find(|fragment| fragment.id == *required_id)
            .cloned()
        {
            sanitized.push(default_fragment);
        }
    }

    if sanitized.is_empty() {
        sanitized = default_bundle.fragments;
    }
    sanitized.sort_by(|left, right| left.id.cmp(&right.id));
    bundle.fragments = sanitized;
}

pub fn add_action_prompt_tags(tags: &mut BTreeSet<String>, action: &crate::actions::ActionDef) {
    insert_prompt_tag(tags, &format!("action_{}", action.name));
    for capability in &action.capabilities {
        insert_prompt_tag(tags, capability);
    }
    let metadata = action.action_metadata();
    insert_prompt_tag(tags, &format!("role_{}", action_role_tag(&metadata.role)));
    insert_prompt_tag(
        tags,
        &format!(
            "integration_{}",
            action_integration_class_tag(&metadata.integration_class)
        ),
    );
    insert_prompt_tag(
        tags,
        &format!(
            "delivery_{}",
            action_delivery_mode_tag(&metadata.delivery_mode)
        ),
    );
    insert_prompt_tag(
        tags,
        &format!(
            "side_effect_{}",
            action_side_effect_tag(&metadata.side_effect_level)
        ),
    );
    if metadata.requires_auth {
        insert_prompt_tag(tags, "requires_auth");
    }
}

fn action_role_tag(role: &crate::actions::ActionRole) -> &'static str {
    match role {
        crate::actions::ActionRole::Trigger => "trigger",
        crate::actions::ActionRole::Delivery => "delivery",
        crate::actions::ActionRole::DataSource => "data_source",
        crate::actions::ActionRole::Mutation => "mutation",
        crate::actions::ActionRole::Inspection => "inspection",
        crate::actions::ActionRole::Orchestration => "orchestration",
        crate::actions::ActionRole::Unknown => "unknown",
    }
}

fn action_integration_class_tag(class: &crate::actions::ActionIntegrationClass) -> &'static str {
    match class {
        crate::actions::ActionIntegrationClass::Internal => "internal",
        crate::actions::ActionIntegrationClass::Messaging => "messaging",
        crate::actions::ActionIntegrationClass::Workspace => "workspace",
        crate::actions::ActionIntegrationClass::Search => "search",
        crate::actions::ActionIntegrationClass::Browser => "browser",
        crate::actions::ActionIntegrationClass::Filesystem => "filesystem",
        crate::actions::ActionIntegrationClass::App => "app",
        crate::actions::ActionIntegrationClass::Code => "code",
        crate::actions::ActionIntegrationClass::Network => "network",
        crate::actions::ActionIntegrationClass::Commerce => "commerce",
        crate::actions::ActionIntegrationClass::Analytics => "analytics",
        crate::actions::ActionIntegrationClass::Media => "media",
        crate::actions::ActionIntegrationClass::Unknown => "unknown",
    }
}

fn action_delivery_mode_tag(mode: &crate::actions::ActionDeliveryMode) -> &'static str {
    match mode {
        crate::actions::ActionDeliveryMode::Immediate => "immediate",
        crate::actions::ActionDeliveryMode::Async => "async",
        crate::actions::ActionDeliveryMode::Conditional => "conditional",
        crate::actions::ActionDeliveryMode::Either => "either",
    }
}

fn action_side_effect_tag(effect: &crate::actions::ActionSideEffectLevel) -> &'static str {
    match effect {
        crate::actions::ActionSideEffectLevel::None => "none",
        crate::actions::ActionSideEffectLevel::Notify => "notify",
        crate::actions::ActionSideEffectLevel::Write => "write",
    }
}

pub fn select_prompt_fragments(
    bundle: &PromptFragmentBundleProfile,
    surface: &str,
    active_tags: &BTreeSet<String>,
    max_tokens: usize,
) -> PromptFragmentSelection {
    let surface = surface.trim();
    let mut candidates = bundle
        .fragments
        .iter()
        .filter(|fragment| {
            fragment.enabled
                && (fragment.surface == surface || fragment.surface == "all")
                && (fragment.always_on
                    || fragment
                        .scope_tags
                        .iter()
                        .any(|tag| active_tags.contains(tag)))
        })
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .always_on
            .cmp(&left.always_on)
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut selected = Vec::new();
    let mut evicted = Vec::new();
    let mut used = 0usize;
    for fragment in candidates {
        let cost = fragment.est_tokens.max(estimate_tokens(&fragment.body));
        if !fragment.always_on && max_tokens > 0 && used.saturating_add(cost) > max_tokens {
            evicted.push(fragment.id);
            continue;
        }
        used = used.saturating_add(cost);
        selected.push(fragment);
    }

    PromptFragmentSelection {
        bundle_version: bundle.version.clone(),
        active_tags: active_tags.clone(),
        fragments: selected,
        estimated_tokens: used,
        evicted_fragment_ids: evicted,
    }
}

pub fn prompt_fragment_selection_for_prompt(
    selection: &PromptFragmentSelection,
) -> serde_json::Value {
    serde_json::json!({
        "bundle_version": selection.bundle_version.clone(),
        "active_tags": selection.active_tags.iter().cloned().collect::<Vec<_>>(),
        "estimated_tokens": selection.estimated_tokens,
        "evicted_fragment_ids": selection.evicted_fragment_ids.clone(),
        "fragments": selection.fragments.iter().map(|fragment| {
            serde_json::json!({
                "id": fragment.id.clone(),
                "body": fragment.body.clone(),
            })
        }).collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_always_on_and_matching_fragments_only() {
        let bundle = default_prompt_fragment_bundle();
        let mut tags = BTreeSet::new();
        insert_prompt_tag(&mut tags, "app_hosting");

        let selected = select_prompt_fragments(&bundle, "agent_loop", &tags, 4_000);
        let ids = selected
            .fragments
            .iter()
            .map(|fragment| fragment.id.as_str())
            .collect::<Vec<_>>();

        assert!(ids.contains(&"fragment.baseline.identity_security"));
        assert!(ids.contains(&"fragment.baseline.semantic_dag_contract"));
        assert!(ids.contains(&"fragment.app_delivery.protocol"));
        assert!(!ids.contains(&"fragment.attachments.vision_documents"));
    }

    #[test]
    fn action_tags_are_derived_from_internal_metadata() {
        let action = crate::actions::ActionDef {
            name: "service_manage".to_string(),
            capabilities: vec!["app_hosting".to_string(), "service_management".to_string()],
            ..Default::default()
        };
        let mut tags = BTreeSet::new();
        add_action_prompt_tags(&mut tags, &action);

        assert!(tags.contains("action_service_manage"));
        assert!(tags.contains("app_hosting"));
        assert!(tags.contains("integration_app"));
    }

    #[test]
    fn app_delivery_fragment_prefers_static_before_dynamic_runtime() {
        let bundle = default_prompt_fragment_bundle();
        let fragment = bundle
            .fragments
            .iter()
            .find(|fragment| fragment.id == "fragment.app_delivery.protocol")
            .expect("default app delivery fragment should exist");

        assert!(fragment.body.contains("standalone static/browser bundle"));
        assert!(fragment.body.contains("Use a dynamic backend/runtime only"));
        assert!(fragment
            .body
            .contains("durable jobs that must continue with no browser open"));
    }
}
