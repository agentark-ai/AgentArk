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
    ]
    .iter()
    .any(|needle| lc.contains(needle));

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
    ]
    .iter()
    .any(|needle| lc.contains(needle));

    help_intent && product_topic
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
    if ["settings", "where do i", "where is", "navigation"]
        .iter()
        .any(|needle| lc.contains(needle))
    {
        topics.push("settings");
    }
    if ["models", "provider", "llm"].iter().any(|needle| lc.contains(needle)) {
        topics.push("models");
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

    if topics.is_empty() {
        topics.push("general");
    }

    topics.sort_unstable();
    topics.dedup();
    topics
}

pub fn build_seed_knowledge_items(actions: &[ActionDef]) -> Vec<SeedKnowledgeItem> {
    let mut items = bundled_docs();
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
    fn builds_seed_docs_with_runtime_catalog() {
        let items = build_seed_knowledge_items(&[
            action("gmail_scan", "Read Gmail messages", &["gmail"]),
            action("moltbook", "Interact with Moltbook", &["network"]),
        ]);
        assert!(items.iter().any(|item| item.source == CURATED_SOURCE));
        assert!(items.iter().any(|item| item.source == RUNTIME_SOURCE));
    }
}
