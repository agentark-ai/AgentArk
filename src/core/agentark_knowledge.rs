use crate::actions::ActionDef;
use crate::docs::agentark_manual::{render_agentark_manual_doc, AGENTARK_MANUAL_DOCS};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const CURATED_SOURCE: &str = "agentark_manual";
pub const RUNTIME_SOURCE: &str = "agentark_capabilities";
pub const DOCUMENT_ID_PREFIX: &str = "agentark_knowledge:";
pub const DOCUMENT_CONTENT_TYPE: &str = "application/x-agentark-knowledge";
pub const INTERNAL_DOCUMENT_CONTENT_TYPE_PREFIX: &str = "application/x-agentark-";
const AGENTARK_KNOWLEDGE_CHUNK_MAX_CHARS: usize = 1_400;

pub fn ark_core_product_ontology_prompt() -> &'static str {
    "Ark Core product ontology.\n\n\
     Ark Core is the AgentArk navigation group for the five core operating surfaces. These are product concepts, not generic nouns, when the user's surrounding intent is about AgentArk, this runtime, its UI, its health, its memory, its learning, its supervision, or its recent work.\n\n\
     Ark Core product glossary:\n\
     - Pulse | Operational health, runtime diagnostics, findings, security posture, storage and integration checks, remediation guidance, and safe fix execution.\n\
     - Sentinel | Supervision, follow-ups, routines, observations, approvals, policy enforcement, background proposals, and background learning status.\n\
     - Evolve | Learning lifecycle for prompts, routing, policies, specialist behavior, tests, canaries, impact, review, promotion, and rollback.\n\
     - Memory | Durable facts, preferences, user data, reusable knowledge, provenance, review, retention, and rollback.\n\
     - Reflect | Retrospectives for days, weeks, or months, including work clusters, source coverage, activity rhythm, narrative recap, and background-agent activity.\n\n\
     If a user asks what one of these surfaces does, answer directly from this ontology and add the most useful next step in the UI. If the user is clearly asking about a non-AgentArk meaning, follow that surrounding context instead. Do not ask for generic clarification merely because a product surface name is also an ordinary word."
}

pub fn is_agentark_knowledge_document_id(value: &str) -> bool {
    value.trim().starts_with(DOCUMENT_ID_PREFIX)
}

pub fn is_internal_agentark_document_content_type(value: &str) -> bool {
    value
        .trim()
        .to_ascii_lowercase()
        .starts_with(INTERNAL_DOCUMENT_CONTENT_TYPE_PREFIX)
}

#[derive(Debug, Clone)]
pub struct SeedKnowledgeItem {
    pub title: String,
    pub content: String,
    pub source: &'static str,
    pub url: Option<String>,
    pub tags: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SeedAgentArkKnowledgeDocument {
    pub id: String,
    pub filename: String,
    pub content_type: &'static str,
    pub source: &'static str,
    pub title: String,
    pub content: String,
    pub url: Option<String>,
    pub tags: Option<String>,
    pub chunks: Vec<String>,
}

fn branded_product_text(text: &str) -> String {
    crate::branding::brand_text(text)
}

pub fn build_seed_knowledge_items(actions: &[ActionDef]) -> Vec<SeedKnowledgeItem> {
    let mut items = build_runtime_action_catalog_docs(actions);
    items.extend(build_ui_topology_docs());
    items.extend(build_connect_flow_docs());
    items.extend(bundled_docs());
    items
}

pub fn build_seed_agentark_knowledge_documents(
    actions: &[ActionDef],
) -> Vec<SeedAgentArkKnowledgeDocument> {
    build_seed_knowledge_items(actions)
        .into_iter()
        .map(seed_agentark_knowledge_document)
        .collect()
}

fn seed_agentark_knowledge_document(item: SeedKnowledgeItem) -> SeedAgentArkKnowledgeDocument {
    let id = agentark_knowledge_document_id(&item);
    let filename = agentark_knowledge_filename(&item);
    let chunks = agentark_knowledge_chunks(&item);
    SeedAgentArkKnowledgeDocument {
        id,
        filename,
        content_type: DOCUMENT_CONTENT_TYPE,
        source: item.source,
        title: item.title,
        content: item.content,
        url: item.url,
        tags: item.tags,
        chunks,
    }
}

fn agentark_knowledge_document_id(item: &SeedKnowledgeItem) -> String {
    let mut hasher = Sha256::new();
    hasher.update(item.source.as_bytes());
    hasher.update(b"\n");
    hasher.update(item.title.as_bytes());
    hasher.update(b"\n");
    if let Some(url) = item.url.as_deref() {
        hasher.update(url.as_bytes());
    }
    let digest = hasher.finalize();
    let hash = digest
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{DOCUMENT_ID_PREFIX}{hash}")
}

fn agentark_knowledge_filename(item: &SeedKnowledgeItem) -> String {
    let source = item.source.replace('_', "-");
    let mut slug = item
        .title
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-');
    format!("{}-{}.md", source, slug)
}

fn agentark_knowledge_chunks(item: &SeedKnowledgeItem) -> Vec<String> {
    let header = agentark_knowledge_chunk_header(item);
    let max_body_chars =
        AGENTARK_KNOWLEDGE_CHUNK_MAX_CHARS.saturating_sub(header.chars().count() + 2);
    let max_body_chars = max_body_chars.max(400);
    let mut chunks = Vec::new();
    let mut current = String::new();

    for block in agentark_knowledge_content_blocks(&item.content, max_body_chars) {
        let separator = if current.is_empty() { "" } else { "\n\n" };
        let candidate_len =
            current.chars().count() + separator.chars().count() + block.chars().count();
        if !current.is_empty() && candidate_len > max_body_chars {
            chunks.push(format!("{header}\n\n{}", current.trim()));
            current.clear();
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(block.trim());
    }

    if !current.trim().is_empty() {
        chunks.push(format!("{header}\n\n{}", current.trim()));
    }
    if chunks.is_empty() {
        chunks.push(header);
    }
    chunks
}

fn agentark_knowledge_chunk_header(item: &SeedKnowledgeItem) -> String {
    let mut lines = vec![
        format!("title: {}", item.title.trim()),
        format!("source: {}", item.source),
    ];
    if let Some(tags) = item
        .tags
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("tags: {tags}"));
    }
    if let Some(url) = item
        .url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("url: {url}"));
    }
    lines.join("\n")
}

fn agentark_knowledge_content_blocks(content: &str, max_chars: usize) -> Vec<String> {
    let mut blocks = Vec::new();
    for block in content
        .split("\n\n")
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if block.chars().count() <= max_chars {
            blocks.push(block.to_string());
            continue;
        }
        let mut current = String::new();
        for line in block
            .lines()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let separator = if current.is_empty() { "" } else { "\n" };
            let candidate_len =
                current.chars().count() + separator.chars().count() + line.chars().count();
            if !current.is_empty() && candidate_len > max_chars {
                blocks.push(current.trim().to_string());
                current.clear();
            }
            if line.chars().count() > max_chars {
                if !current.trim().is_empty() {
                    blocks.push(current.trim().to_string());
                    current.clear();
                }
                blocks.extend(split_long_agentark_knowledge_text(line, max_chars));
                continue;
            }
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
        if !current.trim().is_empty() {
            blocks.push(current.trim().to_string());
        }
    }
    blocks
}

fn split_long_agentark_knowledge_text(text: &str, max_chars: usize) -> Vec<String> {
    let chars = text.chars().collect::<Vec<_>>();
    chars
        .chunks(max_chars.max(1))
        .map(|chunk| chunk.iter().collect::<String>())
        .collect()
}

fn bundled_docs() -> Vec<SeedKnowledgeItem> {
    AGENTARK_MANUAL_DOCS
        .iter()
        .map(|doc| SeedKnowledgeItem {
            title: branded_product_text(doc.title),
            content: branded_product_text(&render_agentark_manual_doc(doc)),
            source: CURATED_SOURCE,
            url: Some(crate::branding::help_uri(&format!("help/{}", doc.slug))),
            tags: Some(doc.tags.join(", ")),
        })
        .collect()
}

fn build_connect_flow_docs() -> Vec<SeedKnowledgeItem> {
    let mut content = format!(
        "Live integration connect flow snapshot. These are the chat-native integration setups {} can walk a user through without custom docs.\n\n",
        crate::branding::PRODUCT_NAME,
    );
    for spec in crate::core::connect_flow::all_specs() {
        let required = match spec.required.kind {
            crate::core::connect_flow::SecretRequirementKind::All => {
                format!("required secrets: {}", spec.required.keys.join(", "))
            }
            crate::core::connect_flow::SecretRequirementKind::Any => {
                format!("provide at least one of: {}", spec.required.keys.join(", "))
            }
        };
        let optional = if spec.optional.is_empty() {
            String::new()
        } else {
            format!(" | optional: {}", spec.optional.join(", "))
        };
        content.push_str(&format!(
            "- {} (`{}`) | triggers: {} | {}{}\n",
            spec.name,
            spec.id,
            spec.triggers.join(", "),
            required,
            optional
        ));
    }

    vec![SeedKnowledgeItem {
        title: "Live integration connect flows".to_string(),
        content,
        source: RUNTIME_SOURCE,
        url: Some(crate::branding::help_uri("help/runtime/connect-flows")),
        tags: Some("integrations, setup, secrets, gmail, google_workspace, moltbook".to_string()),
    }]
}

fn build_ui_topology_docs() -> Vec<SeedKnowledgeItem> {
    let mut items = Vec::new();

    let main_nav = [
        (
            "Mission Control",
            "/home",
            "Landing overview and control center.",
        ),
        ("Chat", "/chat", "Primary chat and execution workspace."),
        (
            "Library",
            "/library",
            "Reusable surfaces grouping Skills, Documents, and Apps.",
        ),
        (
            "Skills",
            "/skills",
            "Reusable skills, imports, tests, enable/disable state, security review, and marketplaces.",
        ),
        (
            "Apps",
            "/apps",
            "Built artifacts, deployments, and public links.",
        ),
        (
            "Agents",
            "/swarm",
            "Specialist agent roster and swarm controls.",
        ),
        (
            "Goals",
            "/goals",
            "Long-running intent and outcome tracking.",
        ),
        (
            "Tasks",
            "/tasks",
            "Durable queue, schedules, and approvals.",
        ),
        (
            "Sentinel",
            "/sentinel",
            "Decision inbox for proposals, observations, approvals, and Background learning status.",
        ),
        (
            "Evolve",
            "/evolution",
            "Learning status, review-only suggestions, live tests, stable changes, and rollback.",
        ),
        (
            "Memory",
            "/arkmemory",
            "Stored facts, preferences, user data, knowledge, provenance, review, and rollback.",
        ),
        (
            "Reflect",
            "/arkreflect",
            "Day, week, and month recaps with local work patterns and follow-ups.",
        ),
        (
            "Watchers",
            "/watchers",
            "Background poll-until-condition monitors.",
        ),
        (
            "Pulse",
            "/arkpulse",
            "Operational health, findings, and fix guidance.",
        ),
        ("Trace", "/trace", "Execution history and tool telemetry."),
        (
            "Documents",
            "/documents",
            "Uploaded files and indexed document context.",
        ),
        ("Analytics", "/analytics", "Usage and analytics dashboards."),
        (
            "Settings",
            "/settings",
            "Modal settings surface for setup and admin controls.",
        ),
    ];
    let mut nav_content = format!(
        "Current {} main navigation and top-level product surfaces.\n\n",
        crate::branding::PRODUCT_NAME,
    );
    for (label, route, detail) in main_nav {
        nav_content.push_str(&format!("- {} (`{}`) | {}\n", label, route, detail));
    }
    items.push(SeedKnowledgeItem {
        title: "Main navigation and top-level pages".to_string(),
        content: nav_content,
        source: RUNTIME_SOURCE,
        url: Some(crate::branding::help_uri("help/runtime/main-navigation")),
        tags: Some(
            "navigation, ui, routes, capabilities, chat, library, documents, tasks, watchers, apps, goals, trace, analytics, settings, swarm, skills"
                .to_string(),
        ),
    });

    let settings_groups: [(&str, &[&str]); 5] = [
        (
            "Setup",
            &[
                "Settings > General",
                "Settings > Models",
                "Settings > Media",
            ],
        ),
        (
            "Integrations",
            &[
                "Settings > Integrations > Messaging Channels",
                "Settings > Integrations > Prebuilt Connectors",
                "Settings > Integrations > Custom Integrations",
                "Settings > Integrations > MCP Servers",
                "Settings > Integrations > Webhooks & APIs",
                "Settings > Integrations > Plugins",
            ],
        ),
        ("Knowledge", &["Memory"]),
        (
            "Admin",
            &["Settings > Data Lifecycle", "Settings > Observability"],
        ),
        ("Security", &["Settings > Security", "Settings > Advanced"]),
    ];
    let mut settings_content = format!(
        "Current Settings navigation groups and tabs in {}.\n\n",
        crate::branding::PRODUCT_NAME,
    );
    for (group, tabs) in settings_groups {
        settings_content.push_str(&format!("- {} | {}\n", group, tabs.join(" | ")));
    }
    items.push(SeedKnowledgeItem {
        title: "Settings groups and tabs".to_string(),
        content: settings_content,
        source: RUNTIME_SOURCE,
        url: Some(crate::branding::help_uri("help/runtime/settings-navigation")),
        tags: Some(
            "settings, navigation, models, integrations, channels, connectors, knowledge, memory, evolution, security, advanced, observability, mcp"
                .to_string(),
        ),
    });

    let cwd = std::env::current_dir()
        .ok()
        .map(|dir| dir.display().to_string())
        .unwrap_or_else(|| ".".to_string());
    let cpu_count = std::thread::available_parallelism()
        .map(|value| value.get().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let docker_host = std::env::var("DOCKER_HOST")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let container_runtime_available =
        docker_host.is_some() || std::path::Path::new("/var/run/docker.sock").exists();
    let mut environment_content = format!(
        "Runtime environment and investigation snapshot for this {} process.\n\n\
- Host view | {} / {}\n\
- Current workspace | `{}`\n\
- Visible logical CPUs | {}\n\
- Container runtime configured | {}\n",
        crate::branding::PRODUCT_NAME,
        std::env::consts::OS,
        std::env::consts::ARCH,
        cwd,
        cpu_count,
        if container_runtime_available {
            "yes"
        } else {
            "no"
        }
    );
    if let Some(host) = docker_host {
        environment_content.push_str(&format!("- Docker routing clue | `{}`\n", host));
    }
    if std::path::Path::new("/app/data/apps").exists() {
        environment_content.push_str(
            "- Managed app root clue | `/app/data/apps/<id>` is present in this process view.\n",
        );
    } else {
        environment_content.push_str(
            "- Managed app root clue | `/app/data/apps/<id>` is not present in this process view.\n",
        );
    }
    environment_content.push_str(
        "\nInvestigation guidance.\n\n\
- Use the request-scoped runtime access summary and AgentArk capability registry as the live source of truth for what this instance can do right now.\n\
- Use integration inventory for connected channels and connectors.\n\
- Use MCP Servers and Plugins settings when the user asks about external capability extensions.\n\
- Use Tasks, Watchers, Goals, Apps, Trace, Analytics, and Pulse to inspect durable work and operational state.\n\
- Use Security and approval-related surfaces to understand what still needs approval.\n\
- If exact host RAM or orchestrator memory ceilings matter, verify from the live deployment/runtime layer rather than guessing from static docs.",
    );
    items.push(SeedKnowledgeItem {
        title: "Runtime environment and investigation".to_string(),
        content: branded_product_text(&environment_content),
        source: RUNTIME_SOURCE,
        url: Some(crate::branding::help_uri("help/runtime/environment")),
        tags: Some(
            "environment, deployment, runtime, workspace, cpu, memory, permissions, approvals, integrations, mcp, plugins, observability, sandbox, docker"
                .to_string(),
        ),
    });

    let data_contract_content = format!(
        "Data ownership contract for release updates in __PRODUCT_NAME__.\n\n\
- User-owned | {}\n\
- System-owned | {}\n\
- Rule | {}",
        crate::core::data_contract::USER_OWNED_SURFACES.join(", "),
        crate::core::data_contract::SYSTEM_OWNED_SURFACES.join(", "),
        crate::core::data_contract::RELEASE_UPDATE_RULE,
    );
    items.push(SeedKnowledgeItem {
        title: "User/system data contract".to_string(),
        content: branded_product_text(&data_contract_content),
        source: RUNTIME_SOURCE,
        url: Some(crate::branding::help_uri("help/runtime/data-contract")),
        tags: Some(
            "data_contract, data_ownership, docker, persistence, updates, upgrades, settings, memory, skills"
                .to_string(),
        ),
    });

    let library_content = "Library and knowledge-related surfaces in the current UI.\n\n\
- Library > Skills | Reusable AgentArk procedures/capabilities, SKILL.md imports, tests, enable/disable state, security review, and marketplaces.\n\
- Library > Documents | Uploaded files and indexed document context.\n\
- Memory | Structured memory, source attribution, review, rollback, and reusable knowledge-base items.\n\
- Memory > Current Memory > Facts | Learned facts and operating constraints captured by the memory system.\n\
- Memory > Current Memory > Preferences | Durable user preferences and rules.\n\
- Memory > Current Memory > User Data | Notes, links, and captured user data.\n\
- Memory > Current Memory > Knowledge | Reusable knowledge-base items, including AgentArk manual/capability entries after sync.\n\n\
Semantic routing guidance.\n\n\
- Choose by meaning, not by product-name usage or keyword matching.\n\
- Skills owns reusable procedures/capabilities and skill marketplaces; explicit \"save as skill\" requests should land here only when the requested item is a workflow or capability, not a durable fact.\n\
- Memory owns persistent personal/work knowledge: durable facts, preferences, user data, source attribution, and reusable knowledge-base items.\n\
- Library > Documents owns file-centric retrieval: uploaded files, indexed documents, document search, and document-grounded answers.";
    items.push(SeedKnowledgeItem {
        title: "Library, documents, and memory surfaces".to_string(),
        content: branded_product_text(library_content),
        source: RUNTIME_SOURCE,
        url: Some(crate::branding::help_uri("help/runtime/library-memory")),
        tags: Some(
            "library, documents, memory, knowledge, files, uploads, facts, preferences, user_data"
                .to_string(),
        ),
    });

    let automation_content = "Automation-oriented surfaces in __PRODUCT_NAME__.\n\n\
- Tasks (`/tasks`) | One-off and recurring work, approvals, and queue state.\n\
- Watchers (`/watchers`) | Bounded poll-until-condition workflows with timeout and trigger state.\n\
- Goals (`/goals`) | Longer-running intent and outcome loops.\n\
- Apps (`/apps`) | App builds, deployments, restore state, and public links.\n\
- Trace (`/trace`) | Execution traces for what the agent actually did.\n\
- Analytics (`/analytics`) | Aggregated usage and performance dashboards.";
    items.push(SeedKnowledgeItem {
        title: "Automation surfaces: tasks, watchers, goals, apps, trace, analytics".to_string(),
        content: branded_product_text(automation_content),
        source: RUNTIME_SOURCE,
        url: Some(crate::branding::help_uri(
            "help/runtime/automation-surfaces",
        )),
        tags: Some(
            "tasks, watchers, goals, apps, trace, analytics, automation, operations".to_string(),
        ),
    });

    let evolution_content = "Evolve and self-learning surfaces in the current UI.\n\n\
- Evolve | Main self-learning page with Overview, Results, Live tests, Review queue, and developer controls.\n\
- Sentinel > Background learning | Live status for heuristic reflection, experience consolidation, pattern induction, and candidate generation.\n\
- Learned Heuristics | Short transferable lessons distilled from completed runs.\n\
- Evolve > Overview | Current state, whether behavior changed, next step, and rollback availability.\n\
- Evolve > Results | Measured impact from prompt, classifier, specialist, and routing changes.\n\
- Evolve > Live tests | Canary rollout, baseline version, candidate version, and gate result for each evolvable surface.\n\
- Evolve > Review queue | Draft workflow, strategy, and memory candidates waiting for review.\n\
- Settings > Advanced > Sentinel | Keep Sentinel available, choose whether it watches AgentArk activity or connected apps, and control routine detection.\n\
- Settings > Advanced > Evolve | Evolve self-evolve master switch for background learning and canary experiments.\n\
- Settings > Advanced > App Deploy Defaults | Default app access guard for new app deploy and public-link flows.\n\
- Evolve > Controls | Developer-mode canary actions and manual testing controls.\n\
- Learned Memory | Durable facts, rules, lessons, and memory extracted from runs.\n\
- Learned Procedures | Repeated successful workflows distilled into procedures.\n\
- Recent Experience Runs | Recent evidence feeding the learning system.\n\
- Learning Candidates | Draft workflow/strategy/memory actions waiting for review.\n\
- Canary History and Strategy Metrics | Diagnostics for policy rollout and promotion decisions.\n\n\
Semantic routing guidance.\n\n\
- Choose by meaning, not by product-name usage or keyword matching.\n\
- Evolve owns the improvement lifecycle: learning state, experiments, canary or live tests, stable behavior changes, deployment uncertainty, rollback state, and self-evolve controls.\n\
- Sentinel owns operator decision state: approvals, rejected or snoozed suggestions, background observations, and items waiting for user attention.\n\
- Pulse owns operational health: diagnostics, findings, runtime state, remediation guidance, and safe fix execution.\n\
- Reflect owns retrospective understanding: time-window recaps, work patterns, source coverage, activity rhythm, and background-agent activity summaries.";
    items.push(SeedKnowledgeItem {
        title: "Evolve and self-learning surfaces".to_string(),
        content: evolution_content.to_string(),
        source: RUNTIME_SOURCE,
        url: Some(crate::branding::help_uri("help/runtime/evolution")),
        tags: Some(
            "self_learning, evolution, learning, background_learning, sentinel, canary, replay_gate, memory, procedures, candidates, heuristics, erl, settings"
                .to_string(),
        ),
    });

    items
}

fn build_runtime_action_catalog_docs(actions: &[ActionDef]) -> Vec<SeedKnowledgeItem> {
    let mut sorted = actions.to_vec();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));

    let mut items = Vec::new();
    let chunk_size = 16;
    for (idx, chunk) in sorted.chunks(chunk_size).enumerate() {
        let mut content = format!(
            "Live AgentArk capability registry snapshot. If an action appears here, this {} instance can use it when the request matches. Required credentials/config for listed actions have already been satisfied for this snapshot. This registry is authoritative for current capability availability; curated manual docs are supplemental explanation only.\n\n",
            crate::branding::PRODUCT_NAME,
        );
        for action in chunk {
            let caps = if action.capabilities.is_empty() {
                "none".to_string()
            } else {
                action.capabilities.join(", ")
            };
            let metadata = action.action_metadata();
            let auth = if metadata.requires_auth || action.authorization.requires_auth {
                "auth/config satisfied"
            } else {
                "no explicit auth"
            };
            content.push_str(&format!(
                "- `{}` | capabilities: {} | source: {:?} | role: {:?} | integration: {:?} | delivery: {:?} | side_effect: {:?} | auth: {} | {}\n",
                action.name,
                caps,
                action.source,
                metadata.role,
                metadata.integration_class,
                metadata.delivery_mode,
                metadata.side_effect_level,
                auth,
                action.description
            ));
        }
        items.push(SeedKnowledgeItem {
            title: format!("Live capability registry {}", idx + 1),
            content,
            source: RUNTIME_SOURCE,
            url: Some(crate::branding::help_uri(&format!(
                "help/runtime/capabilities-{}",
                idx + 1
            ))),
            tags: Some(action_chunk_tags(chunk).join(", ")),
        });
    }
    items
}

fn action_chunk_tags(actions: &[ActionDef]) -> Vec<String> {
    let mut tags = BTreeSet::new();
    tags.insert("actions".to_string());
    tags.insert("capabilities".to_string());
    tags.insert("runtime".to_string());

    for action in actions {
        for cap in &action.capabilities {
            tags.insert(cap.to_ascii_lowercase());
        }
        let name = action.name.to_ascii_lowercase();
        let desc = action.description.to_ascii_lowercase();
        let combined = format!("{} {}", name, desc);
        for (needle, tag) in [
            ("gmail", "gmail"),
            ("google_workspace", "google_workspace"),
            ("google workspace", "google_workspace"),
            ("moltbook", "moltbook"),
            ("watcher", "watchers"),
            ("schedule", "tasks"),
            ("app", "apps"),
            ("browser", "browser"),
            ("telegram", "channels"),
            ("whatsapp", "channels"),
            ("slack", "channels"),
            ("discord", "channels"),
            ("teams", "channels"),
        ] {
            if combined.contains(needle) {
                tags.insert(tag.to_string());
            }
        }
    }

    tags.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(name: &str, description: &str, capabilities: &[&str]) -> ActionDef {
        ActionDef {
            name: name.to_string(),
            description: description.to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({}),
            capabilities: capabilities.iter().map(|cap| cap.to_string()).collect(),
            sandbox_mode: None,
            source: crate::actions::ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        }
    }

    #[test]
    fn internal_document_content_type_namespace_covers_legacy_help_docs() {
        assert!(is_internal_agentark_document_content_type(
            DOCUMENT_CONTENT_TYPE
        ));
        assert!(is_internal_agentark_document_content_type(
            "application/x-agentark-product-help"
        ));
        assert!(!is_internal_agentark_document_content_type(
            "application/pdf"
        ));
        assert!(!is_internal_agentark_document_content_type("text/markdown"));
    }

    #[test]
    fn builds_seed_docs_with_runtime_catalog() {
        let items = build_seed_knowledge_items(&[
            action("gmail_scan", "Read Gmail messages", &["gmail"]),
            action("moltbook", "Interact with Moltbook", &["network"]),
        ]);
        assert!(items.iter().any(|item| item.source == CURATED_SOURCE));
        assert!(items.iter().any(|item| item.source == RUNTIME_SOURCE));
        assert!(items
            .iter()
            .any(|item| item.title.contains("Main navigation")));
        assert!(items
            .iter()
            .any(|item| item.title == "Models and provider setup"));
        assert!(items
            .iter()
            .any(|item| item.title == "Embeddings and retrieval"));
        assert!(items
            .iter()
            .any(|item| item.title == "Input needed and unattended runs"));
        assert!(items
            .iter()
            .any(|item| item.title == "Environment, deployment, and investigation"));
        assert!(items
            .iter()
            .any(|item| item.title == "Chat shortcuts and safe actions"));
        assert!(items
            .iter()
            .any(|item| item.title == "Custom integrations and extension packs"));
        assert!(items
            .iter()
            .any(|item| item.title == "MCP servers, plugins, webhooks, and custom APIs"));
        assert!(items
            .iter()
            .any(|item| item.title == "Runtime environment and investigation"));
    }
}
