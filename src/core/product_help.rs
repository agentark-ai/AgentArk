use crate::actions::ActionDef;
use std::collections::BTreeSet;

pub const CURATED_SOURCE: &str = "agentark_help";
pub const RUNTIME_SOURCE: &str = "agentark_runtime_help";

#[derive(Debug, Clone)]
pub struct SeedKnowledgeItem {
    pub title: String,
    pub content: String,
    pub source: &'static str,
    pub url: Option<String>,
    pub tags: Option<String>,
}

struct BundledHelpDoc {
    title: &'static str,
    slug: &'static str,
    tags: &'static [&'static str],
    content: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductHelpMode {
    Setup,
    Explain,
    Status,
}

impl ProductHelpMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Explain => "explain",
            Self::Status => "status",
        }
    }
}

const BUNDLED_DOCS: &[BundledHelpDoc] = &[
    BundledHelpDoc {
        title: "Install and first run",
        slug: "install-and-first-run",
        tags: &["install", "first_run", "new_user", "models", "setup"],
        content: include_str!("product_help_docs/install-and-first-run.md"),
    },
    BundledHelpDoc {
        title: "Settings and navigation map",
        slug: "settings-and-navigation",
        tags: &["settings", "navigation", "ui", "paths"],
        content: include_str!("product_help_docs/settings-and-navigation.md"),
    },
    BundledHelpDoc {
        title: "Mission Control, chat, and approvals",
        slug: "mission-control-chat-and-approvals",
        tags: &["mission_control", "chat", "inbox", "approvals", "navigation"],
        content: include_str!("product_help_docs/mission-control-chat-and-inbox.md"),
    },
    BundledHelpDoc {
        title: "Models and provider setup",
        slug: "models-and-provider-setup",
        tags: &["models", "providers", "llm", "setup", "routing", "research"],
        content: include_str!("product_help_docs/models-and-provider-setup.md"),
    },
    BundledHelpDoc {
        title: "Media generation providers",
        slug: "media-generation-providers",
        tags: &["media", "images", "video", "providers", "settings", "api_keys"],
        content: include_str!("product_help_docs/media-generation-providers.md"),
    },
    BundledHelpDoc {
        title: "Messaging channels and daily brief",
        slug: "messaging-channels-and-daily-brief",
        tags: &[
            "channels",
            "telegram",
            "slack",
            "discord",
            "matrix",
            "teams",
            "whatsapp",
            "daily_brief",
            "setup",
        ],
        content: include_str!("product_help_docs/messaging-channels-and-daily-brief.md"),
    },
    BundledHelpDoc {
        title: "Prebuilt connectors and integration quickstarts",
        slug: "prebuilt-connectors-and-integration-quickstarts",
        tags: &["integrations", "connectors", "oauth", "setup", "status"],
        content: include_str!("product_help_docs/prebuilt-connectors-and-integration-quickstarts.md"),
    },
    BundledHelpDoc {
        title: "Add Gmail access through Google Workspace",
        slug: "gmail-google-workspace",
        tags: &[
            "gmail",
            "google_workspace",
            "google_cloud",
            "oauth",
            "integrations",
            "setup",
        ],
        content: include_str!("product_help_docs/gmail-google-workspace.md"),
    },
    BundledHelpDoc {
        title: "Run Moltbook for the first time",
        slug: "moltbook-first-run",
        tags: &["moltbook", "social", "integrations", "setup", "run"],
        content: include_str!("product_help_docs/moltbook.md"),
    },
    BundledHelpDoc {
        title: "Library, memory, documents, and MCP",
        slug: "library-memory-documents-and-mcp",
        tags: &[
            "library",
            "documents",
            "memory",
            "knowledge",
            "mcp",
            "facts",
            "preferences",
            "user_data",
        ],
        content: include_str!("product_help_docs/library-memory-documents-and-mcp.md"),
    },
    BundledHelpDoc {
        title: "Tasks, watchers, goals, and apps",
        slug: "tasks-watchers-goals-and-apps",
        tags: &["tasks", "watchers", "goals", "apps", "automation", "deploy"],
        content: include_str!("product_help_docs/tasks-watchers-goals-and-apps.md"),
    },
    BundledHelpDoc {
        title: "App deploy and access guard",
        slug: "app-deploy-and-access-guard",
        tags: &[
            "apps",
            "deploy",
            "app_deploy",
            "access_guard",
            "public_apps",
            "security",
        ],
        content: include_str!("product_help_docs/app-deploy-and-access-guard.md"),
    },
    BundledHelpDoc {
        title: "Trace, analytics, and ArkPulse",
        slug: "trace-analytics-and-arkpulse",
        tags: &["trace", "analytics", "arkpulse", "observability", "operations"],
        content: include_str!("product_help_docs/trace-analytics-and-arkpulse.md"),
    },
    BundledHelpDoc {
        title: "Self-learning and evolution",
        slug: "self-learning-and-evolution",
        tags: &[
            "self_learning",
            "evolution",
            "learning",
            "memory",
            "canary",
            "settings",
        ],
        content: include_str!("product_help_docs/self-learning-and-evolution.md"),
    },
    BundledHelpDoc {
        title: "Plugins, webhooks, and custom APIs",
        slug: "plugins-webhooks-and-custom-apis",
        tags: &[
            "plugins",
            "webhooks",
            "custom_api",
            "integrations",
            "mcp",
            "events",
        ],
        content: include_str!("product_help_docs/plugins-webhooks-and-custom-apis.md"),
    },
    BundledHelpDoc {
        title: "Security, advanced settings, and secrets",
        slug: "security-advanced-and-secrets",
        tags: &[
            "security",
            "advanced",
            "secrets",
            "master_password",
            "sender_verification",
        ],
        content: include_str!("product_help_docs/security-advanced-and-secrets.md"),
    },
    BundledHelpDoc {
        title: "Swarm, agents, and delegation",
        slug: "swarm-agents-and-delegation",
        tags: &["swarm", "agents", "delegation", "specialists", "multi_agent"],
        content: include_str!("product_help_docs/swarm-agents-and-delegation.md"),
    },
    BundledHelpDoc {
        title: "Browser automation, search, and research",
        slug: "browser-search-and-research",
        tags: &["browser", "search", "research", "web_search", "browser_auto", "chat"],
        content: include_str!("product_help_docs/browser-search-and-research.md"),
    },
    BundledHelpDoc {
        title: "AgentArk capabilities overview",
        slug: "capabilities-overview",
        tags: &["capabilities", "features", "overview", "general"],
        content: include_str!("product_help_docs/capabilities-overview.md"),
    },
];

pub fn is_product_help_source(source: Option<&str>) -> bool {
    matches!(source, Some(CURATED_SOURCE | RUNTIME_SOURCE))
}

pub fn looks_like_agentark_help_query(message: &str) -> bool {
    let lc = message.trim().to_ascii_lowercase();
    if lc.is_empty() {
        return false;
    }

    let help_intent = [
        "how do i",
        "how to",
        "where do i",
        "where is",
        "what can",
        "show me how",
        "walk me through",
        "steps to",
        "i am new",
        "i'm new",
        "new user",
        "setup",
        "set up",
        "configure",
        "connect",
        "enable",
        "add access",
        "how can i",
        "help me",
        "how does",
        "what is",
        "what's",
        "explain",
        "tell me about",
        "status",
        "state",
        "current",
        "enabled",
        "disabled",
        "working",
    ]
    .iter()
    .any(|needle| lc.contains(needle));
    let help_intent = help_intent
        || lc.starts_with("is ")
        || lc.starts_with("are ")
        || lc.starts_with("does ")
        || lc.starts_with("can ");

    let product_topic = [
        "agentark",
        "gmail",
        "google workspace",
        "gws",
        "google cloud",
        "moltbook",
        "settings",
        "integrations",
        "models",
        "watcher",
        "watchers",
        "tasks",
        "apps",
        "channels",
        "telegram",
        "whatsapp",
        "github",
        "notion",
        "twilio",
        "self learning",
        "self-learning",
        "learning",
        "evolution",
        "memory",
        "knowledge",
        "documents",
        "library",
        "skills",
        "swarm",
        "agents",
        "goals",
        "trace",
        "analytics",
        "arkpulse",
        "security",
        "advanced",
        "browser",
        "mcp",
        "webhooks",
    ]
    .iter()
    .any(|needle| lc.contains(needle));

    help_intent && product_topic
}

pub fn infer_help_mode(message: &str) -> ProductHelpMode {
    let lc = message.trim().to_ascii_lowercase();
    if lc.is_empty() {
        return ProductHelpMode::Setup;
    }

    if [
        "how do i",
        "how to",
        "where do i",
        "set up",
        "setup",
        "configure",
        "connect",
        "add access",
        "walk me through",
        "steps to",
        "new user",
        "i am new",
        "i'm new",
    ]
    .iter()
    .any(|needle| lc.contains(needle))
    {
        return ProductHelpMode::Setup;
    }

    if [
        "status",
        "state",
        "enabled",
        "disabled",
        "connected",
        "configured",
        "current",
        "right now",
        "last run",
        "queue",
        "how many",
    ]
    .iter()
    .any(|needle| lc.contains(needle))
        || lc.starts_with("is ")
        || lc.starts_with("are ")
        || lc.starts_with("does ")
    {
        return ProductHelpMode::Status;
    }

    ProductHelpMode::Explain
}

pub fn infer_help_topics(message: &str) -> Vec<&'static str> {
    let lc = message.trim().to_ascii_lowercase();
    let mut topics = Vec::new();

    if lc.contains("gmail") {
        topics.push("gmail");
    }
    if [
        "google workspace",
        "gws",
        "google cloud",
        "oauth",
        "calendar",
        "drive",
        "docs",
        "sheets",
        "chat",
        "admin sdk",
    ]
    .iter()
    .any(|needle| lc.contains(needle))
    {
        topics.push("google_workspace");
    }
    if lc.contains("moltbook") {
        topics.push("moltbook");
    }
    if [
        "self learning",
        "self-learning",
        "learning",
        "evolution",
        "canary",
        "replay gate",
        "learning candidate",
        "learned memory",
        "learned procedure",
    ]
    .iter()
    .any(|needle| lc.contains(needle))
    {
        topics.push("self_learning");
    }
    if ["install", "first run", "start", "docker", "build from source"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("install");
    }
    if ["new user", "i am new", "i'm new"].iter().any(|needle| lc.contains(needle)) {
        topics.push("new_user");
    }
    if ["what can", "capabilities", "features", "what does"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("capabilities");
    }
    if ["mission control", "chat", "inbox", "approval inbox"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("chat");
    }
    if ["settings", "where do i", "where is", "navigation"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("settings");
    }
    if ["models", "provider", "llm"].iter().any(|needle| lc.contains(needle)) {
        topics.push("models");
    }
    if ["media", "image", "video", "dall-e", "gemini", "veo", "replicate", "runway", "luma"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("media");
    }
    if ["daily brief", "telegram", "slack", "discord", "matrix", "teams", "whatsapp"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("channels");
    }
    if ["memory", "knowledge", "facts", "preferences", "user data"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("memory");
    }
    if ["document", "documents", "upload", "file", "files", "library"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("documents");
    }
    if ["watcher", "watchers", "monitor"].iter().any(|needle| lc.contains(needle)) {
        topics.push("watchers");
    }
    if ["task", "tasks", "schedule"].iter().any(|needle| lc.contains(needle)) {
        topics.push("tasks");
    }
    if ["app", "apps", "deploy"].iter().any(|needle| lc.contains(needle)) {
        topics.push("apps");
    }
    if ["channel", "telegram", "whatsapp", "slack", "discord", "teams"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("channels");
    }
    if ["integrations", "integration", "connectors"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("integrations");
    }
    if ["plugin", "plugins", "plugin sdk"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("plugins");
    }
    if ["webhook", "webhooks", "custom api", "custom apis", "incoming webhook"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("webhooks");
    }
    if ["skills", "skill import", "capability"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("skills");
    }
    if ["swarm", "specialist agent", "specialist agents", "agents page"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("swarm");
    }
    if ["goal", "goals"].iter().any(|needle| lc.contains(needle)) {
        topics.push("goals");
    }
    if ["trace", "execution trace", "logs", "what did it do"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("trace");
    }
    if ["arkpulse", "pulse", "health check", "operational pulse"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("arkpulse");
    }
    if ["analytics", "usage metrics", "llm analytics"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("analytics");
    }
    if ["browser", "website", "form fill", "browser automation"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("browser");
    }
    if ["research", "web search", "deep research", "search the web"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("research");
    }
    if ["security", "advanced", "api key", "mcp", "observability", "webhook"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("security");
    }

    if topics.is_empty() {
        topics.push("general");
    }

    topics.sort_unstable();
    topics.dedup();
    topics
}

pub fn build_seed_knowledge_items(actions: &[ActionDef]) -> Vec<SeedKnowledgeItem> {
    let mut items = bundled_docs();
    items.extend(build_ui_topology_docs());
    items.extend(build_connect_flow_docs());
    items.extend(build_runtime_action_catalog_docs(actions));
    items
}

fn bundled_docs() -> Vec<SeedKnowledgeItem> {
    BUNDLED_DOCS
        .iter()
        .map(|doc| SeedKnowledgeItem {
            title: doc.title.to_string(),
            content: doc.content.trim().to_string(),
            source: CURATED_SOURCE,
            url: Some(format!("agentark://help/{}", doc.slug)),
            tags: Some(doc.tags.join(", ")),
        })
        .collect()
}

fn build_connect_flow_docs() -> Vec<SeedKnowledgeItem> {
    let mut content = String::from(
        "Live integration connect flow snapshot. These are the chat-native integration setups AgentArk can walk a user through without custom docs.\n\n",
    );
    for spec in crate::core::connect_flow::all_specs() {
        let required = match spec.required.kind {
            crate::core::connect_flow::SecretRequirementKind::All => {
                format!("required secrets: {}", spec.required.keys.join(", "))
            }
            crate::core::connect_flow::SecretRequirementKind::Any => {
                format!(
                    "provide at least one of: {}",
                    spec.required.keys.join(", ")
                )
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
        url: Some("agentark://help/runtime/connect-flows".to_string()),
        tags: Some(
            "integrations, setup, secrets, gmail, google_workspace, moltbook".to_string(),
        ),
    }]
}

fn build_ui_topology_docs() -> Vec<SeedKnowledgeItem> {
    let mut items = Vec::new();

    let main_nav = [
        ("Mission Control", "/home", "Landing overview and control center."),
        ("Chat", "/chat", "Primary chat and execution workspace."),
        ("Library", "/library", "Reusable surfaces grouping Skills, Documents, and Apps."),
        ("Skills", "/skills", "Reusable skills and imports."),
        ("Apps", "/apps", "Built artifacts, deployments, and public links."),
        ("Agents", "/swarm", "Specialist agent roster and swarm controls."),
        ("Goals", "/goals", "Long-running intent and outcome tracking."),
        ("Moltbook", "/moltbook", "Top-level Moltbook control page."),
        ("Tasks", "/tasks", "Durable queue, schedules, and approvals."),
        ("Watchers", "/watchers", "Background poll-until-condition monitors."),
        ("ArkPulse", "/arkpulse", "Operational pulse and guidance."),
        ("Trace", "/trace", "Execution history and tool telemetry."),
        ("Documents", "/documents", "Uploaded files and indexed document context."),
        ("Analytics", "/analytics", "Usage and analytics dashboards."),
        ("Settings", "/settings", "Modal settings surface for setup and admin controls."),
    ];
    let mut nav_content =
        String::from("Current AgentArk main navigation and top-level product surfaces.\n\n");
    for (label, route, detail) in main_nav {
        nav_content.push_str(&format!("- {} (`{}`) | {}\n", label, route, detail));
    }
    items.push(SeedKnowledgeItem {
        title: "Main navigation and top-level pages".to_string(),
        content: nav_content,
        source: RUNTIME_SOURCE,
        url: Some("agentark://help/runtime/main-navigation".to_string()),
        tags: Some(
            "navigation, ui, routes, capabilities, chat, library, documents, tasks, watchers, apps, goals, moltbook, trace, analytics, settings, swarm, skills"
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
                "Settings > Integrations > Webhooks & APIs",
                "Settings > Integrations > Plugins",
            ],
        ),
        (
            "Knowledge",
            &[
                "Settings > Knowledge > Memory",
                "Settings > Knowledge > MCP Servers",
            ],
        ),
        (
            "Admin",
            &[
                "Settings > Data Lifecycle",
                "Settings > Observability",
                "Settings > Evolution",
            ],
        ),
        (
            "Security",
            &["Settings > Security", "Settings > Advanced"],
        ),
    ];
    let mut settings_content =
        String::from("Current Settings navigation groups and tabs in AgentArk.\n\n");
    for (group, tabs) in settings_groups {
        settings_content.push_str(&format!("- {} | {}\n", group, tabs.join(" | ")));
    }
    items.push(SeedKnowledgeItem {
        title: "Settings groups and tabs".to_string(),
        content: settings_content,
        source: RUNTIME_SOURCE,
        url: Some("agentark://help/runtime/settings-navigation".to_string()),
        tags: Some(
            "settings, navigation, models, integrations, channels, connectors, knowledge, memory, evolution, security, advanced, observability, mcp"
                .to_string(),
        ),
    });

    let library_content = "Library and knowledge-related surfaces in the current UI.\n\n\
- Library > Documents | Uploaded files and indexed document context.\n\
- Settings > Knowledge > Memory | Structured memory and reusable knowledge-base items.\n\
- Settings > Knowledge > Memory > Facts | Semantic facts extracted or stored by the system.\n\
- Settings > Knowledge > Memory > Preferences | Durable user preferences and rules.\n\
- Settings > Knowledge > Memory > User Data | Notes, links, and captured user data.\n\
- Settings > Knowledge > Memory > Knowledge | Reusable knowledge-base items, including bundled product docs after sync.\n\
- Settings > Knowledge > MCP Servers | External MCP server configuration.";
    items.push(SeedKnowledgeItem {
        title: "Library, documents, and memory surfaces".to_string(),
        content: library_content.to_string(),
        source: RUNTIME_SOURCE,
        url: Some("agentark://help/runtime/library-memory".to_string()),
        tags: Some(
            "library, documents, memory, knowledge, files, uploads, facts, preferences, user_data, mcp"
                .to_string(),
        ),
    });

    let automation_content = "Automation-oriented surfaces in AgentArk.\n\n\
- Tasks (`/tasks`) | One-off and recurring work, approvals, and queue state.\n\
- Watchers (`/watchers`) | Bounded poll-until-condition workflows with timeout and trigger state.\n\
- Goals (`/goals`) | Longer-running intent and outcome loops.\n\
- Apps (`/apps`) | App builds, deployments, restore state, and public links.\n\
- Trace (`/trace`) | Execution traces for what the agent actually did.\n\
- Analytics (`/analytics`) | Aggregated usage and performance dashboards.";
    items.push(SeedKnowledgeItem {
        title: "Automation surfaces: tasks, watchers, goals, apps, trace, analytics".to_string(),
        content: automation_content.to_string(),
        source: RUNTIME_SOURCE,
        url: Some("agentark://help/runtime/automation-surfaces".to_string()),
        tags: Some(
            "tasks, watchers, goals, apps, trace, analytics, automation, operations"
                .to_string(),
        ),
    });

    let evolution_content = "Self-learning and evolution surfaces in the current UI.\n\n\
- Settings > Admin > Evolution | Main self-learning and evolution control center.\n\
- Evolution Status | Self-evolve toggle, learning toggle, local-only mode, canary state, promotion mode, replay gate, queue metrics.\n\
- Deploy Guard Default | Default access-guard behavior for app deployments.\n\
- Learned Memory | Durable facts, rules, lessons, and memory extracted from runs.\n\
- Learned Procedures | Repeated successful workflows distilled into procedures.\n\
- Recent Experience Runs | Recent evidence feeding the learning system.\n\
- Learning Candidates | Draft workflow/strategy/memory actions waiting for review.\n\
- Canary History and Strategy Metrics | Diagnostics for policy rollout and promotion decisions.";
    items.push(SeedKnowledgeItem {
        title: "Evolution and self-learning surfaces".to_string(),
        content: evolution_content.to_string(),
        source: RUNTIME_SOURCE,
        url: Some("agentark://help/runtime/evolution".to_string()),
        tags: Some(
            "self_learning, evolution, learning, canary, replay_gate, memory, procedures, candidates, settings"
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
        let mut content = String::from(
            "Live action snapshot. If an action appears here, this AgentArk instance can use it when the request matches and required credentials/config are present.\n\n",
        );
        for action in chunk {
            let caps = if action.capabilities.is_empty() {
                "none".to_string()
            } else {
                action.capabilities.join(", ")
            };
            content.push_str(&format!(
                "- `{}` | capabilities: {} | {}\n",
                action.name, caps, action.description
            ));
        }
        items.push(SeedKnowledgeItem {
            title: format!("Live action catalog {}", idx + 1),
            content,
            source: RUNTIME_SOURCE,
            url: Some(format!("agentark://help/runtime/actions-{}", idx + 1)),
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
        }
    }

    #[test]
    fn detects_agentark_help_queries() {
        assert!(looks_like_agentark_help_query(
            "How do I add Gmail access in AgentArk?"
        ));
        assert!(looks_like_agentark_help_query(
            "I am new, how do I run Moltbook?"
        ));
        assert!(!looks_like_agentark_help_query(
            "Post on Moltbook about this release"
        ));
    }

    #[test]
    fn infers_help_topics() {
        let topics = infer_help_topics("How do I add Gmail access with Google Workspace?");
        assert!(topics.contains(&"gmail"));
        assert!(topics.contains(&"google_workspace"));
    }

    #[test]
    fn detects_self_learning_help_queries() {
        assert!(looks_like_agentark_help_query(
            "How does self-learning work in AgentArk?"
        ));
        assert!(matches!(
            infer_help_mode("Is self-learning enabled right now?"),
            ProductHelpMode::Status
        ));
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
            .any(|item| item.title == "Plugins, webhooks, and custom APIs"));
    }
}
