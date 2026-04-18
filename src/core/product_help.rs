use crate::actions::ActionDef;
use crate::docs::product_help::{render_bundled_help_doc, BUNDLED_HELP_DOCS};
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

fn branded_product_text(text: &str) -> String {
    crate::branding::brand_text(text)
}

fn normalize_help_text(text: &str) -> String {
    text.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
}

fn normalized_help_tokens(text: &str) -> BTreeSet<String> {
    normalize_help_text(text)
        .split_whitespace()
        .map(|token| token.to_string())
        .collect()
}

pub fn canonical_help_topic(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_separator = false;
    for ch in value.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_was_separator = false;
        } else if !out.is_empty() && !last_was_separator {
            out.push('_');
            last_was_separator = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

pub fn known_help_topics() -> Vec<String> {
    let mut topics = BTreeSet::new();
    topics.insert("general".to_string());
    topics.insert("overview".to_string());
    topics.insert("features".to_string());
    topics.insert("capabilities".to_string());
    for doc in BUNDLED_HELP_DOCS {
        for tag in doc.tags {
            topics.insert(canonical_help_topic(tag));
        }
    }
    topics.into_iter().collect()
}

#[cfg(test)]
fn contains_help_phrase(haystack: &str, phrase: &str) -> bool {
    let haystack = format!(" {} ", normalize_help_text(haystack));
    let needle = format!(" {} ", normalize_help_text(phrase));
    !needle.trim().is_empty() && haystack.contains(&needle)
}

#[cfg(test)]
fn contains_any_help_phrase(haystack: &str, phrases: &[&str]) -> bool {
    phrases
        .iter()
        .any(|phrase| contains_help_phrase(haystack, phrase))
}

#[cfg(test)]
fn contains_any_help_token(haystack: &str, tokens: &[&str]) -> bool {
    let words = normalized_help_tokens(haystack);
    tokens.iter().any(|token| {
        let normalized = token.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return false;
        }
        if normalized
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            words.contains(&normalized)
        } else {
            contains_help_phrase(haystack, &normalized)
        }
    })
}

#[cfg(test)]
fn query_matches_help_intent(message: &str) -> bool {
    const INTENT_PHRASES: &[&str] = &[
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
        "set up",
        "setup",
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
    ];
    const INTENT_TOKENS: &[&str] = &[
        "status", "state", "current", "enabled", "disabled", "working",
    ];

    contains_any_help_phrase(message, INTENT_PHRASES)
        || contains_any_help_token(message, INTENT_TOKENS)
        || matches!(
            normalize_help_text(message).split_whitespace().next(),
            Some("is" | "are" | "does" | "can" | "how" | "what" | "where" | "why")
        )
}

#[cfg(test)]
fn topic_matches(message: &str, phrases: &[&str], tokens: &[&str]) -> bool {
    contains_any_help_phrase(message, phrases) || contains_any_help_token(message, tokens)
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

#[derive(Debug, Clone)]
pub struct BundledHelpMatch {
    pub title: String,
    pub slug: String,
    pub url: String,
    pub tags: Vec<String>,
    pub content: String,
    pub score: usize,
}

fn branded_doc_title(doc: &crate::docs::product_help::BundledHelpDoc) -> String {
    branded_product_text(doc.title)
}

fn branded_doc_summary(doc: &crate::docs::product_help::BundledHelpDoc) -> String {
    branded_product_text(doc.summary)
}

fn branded_doc_content(doc: &crate::docs::product_help::BundledHelpDoc) -> String {
    branded_product_text(&render_bundled_help_doc(doc))
}

fn decamelized_product_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (idx, ch) in name.chars().enumerate() {
        if idx > 0 && ch.is_ascii_uppercase() {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

fn product_reference_aliases() -> Vec<String> {
    let mut aliases = BTreeSet::new();
    aliases.insert(crate::branding::PRODUCT_NAME.to_string());
    aliases.insert(crate::branding::PRODUCT_SLUG.to_string());
    aliases.insert(decamelized_product_name(crate::branding::PRODUCT_NAME));
    if crate::branding::LEGACY_PRODUCT_NAME != crate::branding::PRODUCT_NAME {
        aliases.insert(crate::branding::LEGACY_PRODUCT_NAME.to_string());
        aliases.insert(decamelized_product_name(
            crate::branding::LEGACY_PRODUCT_NAME,
        ));
    }
    aliases.into_iter().collect()
}

fn is_low_signal_help_token(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "and"
            | "agent"
            | "agents"
            | "ai"
            | "am"
            | "are"
            | "as"
            | "assistant"
            | "assistants"
            | "be"
            | "can"
            | "does"
            | "do"
            | "for"
            | "help"
            | "how"
            | "i"
            | "in"
            | "is"
            | "it"
            | "me"
            | "my"
            | "of"
            | "on"
            | "tell"
            | "the"
            | "this"
            | "to"
            | "use"
            | "what"
            | "whats"
            | "with"
    )
}

fn filtered_help_query_tokens(message: &str) -> BTreeSet<String> {
    let product_tokens = product_reference_aliases()
        .into_iter()
        .flat_map(|alias| normalized_help_tokens(&alias).into_iter())
        .collect::<BTreeSet<_>>();

    normalized_help_tokens(message)
        .into_iter()
        .filter(|token| token.len() > 1)
        .filter(|token| !product_tokens.contains(token))
        .filter(|token| !is_low_signal_help_token(token))
        .collect()
}

pub fn help_query_match_tokens(message: &str) -> Vec<String> {
    filtered_help_query_tokens(message).into_iter().collect()
}

fn bundled_doc_tag_tokens(doc: &crate::docs::product_help::BundledHelpDoc) -> BTreeSet<String> {
    doc.tags
        .iter()
        .flat_map(|tag| normalized_help_tokens(tag).into_iter())
        .collect()
}

fn score_bundled_help_doc(
    doc: &crate::docs::product_help::BundledHelpDoc,
    query_tokens: &BTreeSet<String>,
) -> usize {
    if query_tokens.is_empty() {
        return 0;
    }

    let title_tokens = normalized_help_tokens(&branded_doc_title(doc));
    let summary_tokens = normalized_help_tokens(&branded_doc_summary(doc));
    let content_tokens = normalized_help_tokens(&branded_doc_content(doc));
    let tag_tokens = bundled_doc_tag_tokens(doc);

    query_tokens.iter().fold(0usize, |score, token| {
        score
            + if tag_tokens.contains(token) { 8 } else { 0 }
            + if title_tokens.contains(token) { 6 } else { 0 }
            + if summary_tokens.contains(token) { 4 } else { 0 }
            + if content_tokens.contains(token) { 2 } else { 0 }
    })
}

pub fn match_bundled_help_docs(message: &str, limit: usize) -> Vec<BundledHelpMatch> {
    if limit == 0 {
        return Vec::new();
    }

    let query_tokens = filtered_help_query_tokens(message);
    let mut scored = BUNDLED_HELP_DOCS
        .iter()
        .map(|doc| {
            let score = score_bundled_help_doc(doc, &query_tokens);
            (doc, score)
        })
        .collect::<Vec<_>>();

    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.slug.cmp(b.0.slug)));

    let matches = scored
        .iter()
        .filter(|(_, score)| *score > 0)
        .take(limit)
        .map(|(doc, score)| BundledHelpMatch {
            title: branded_doc_title(doc),
            slug: doc.slug.to_string(),
            url: crate::branding::help_uri(&format!("help/{}", doc.slug)),
            tags: doc.tags.iter().map(|tag| tag.to_string()).collect(),
            content: branded_doc_content(doc),
            score: *score,
        })
        .collect::<Vec<_>>();

    matches
}

pub fn bundled_help_doc_match_by_slug(slug: &str, score: usize) -> Option<BundledHelpMatch> {
    BUNDLED_HELP_DOCS
        .iter()
        .find(|doc| doc.slug == slug)
        .map(|doc| BundledHelpMatch {
            title: branded_doc_title(doc),
            slug: doc.slug.to_string(),
            url: crate::branding::help_uri(&format!("help/{}", doc.slug)),
            tags: doc.tags.iter().map(|tag| tag.to_string()).collect(),
            content: branded_doc_content(doc),
            score,
        })
}

pub fn infer_help_topics_from_bundled_matches(matches: &[BundledHelpMatch]) -> Vec<String> {
    let mut topics = BTreeSet::new();
    for matched in matches {
        for tag in &matched.tags {
            topics.insert(tag.to_string());
        }
    }
    topics.into_iter().collect()
}

#[cfg(test)]
pub fn looks_like_agentark_help_query(message: &str) -> bool {
    let lc = message.trim().to_ascii_lowercase();
    if lc.is_empty() {
        return false;
    }
    let help_intent = query_matches_help_intent(message);
    let inferred_topics = infer_help_topics(message);
    let high_signal_topic = inferred_topics.iter().any(|topic| {
        matches!(
            *topic,
            "gmail"
                | "google_workspace"
                | "moltbook"
                | "self_learning"
                | "sentinel"
                | "install"
                | "new_user"
                | "capabilities"
                | "integrations"
                | "custom_integrations"
                | "plugins"
                | "webhooks"
                | "swarm"
                | "trace"
                | "arkpulse"
                | "environment"
                | "browser"
                | "research"
                | "security"
                | "data_contract"
        )
    });
    let explicit_product_signal = contains_any_help_phrase(
        message,
        &[
            crate::branding::PRODUCT_NAME,
            "agentark",
            "agent ark",
            "mission control",
            "approval inbox",
            "background learning",
            "heuristic reflection",
            "learned heuristics",
            "experiential reflective learning",
            "erl heuristics",
            "reflection pass",
            "daily brief",
            "custom integrations",
            "custom integration",
            "extension pack",
            "extension packs",
            "prebuilt connectors",
            "plugin sdk",
            "google workspace",
            "skill import",
            "embedding provider",
            "local embeddings",
            "external embeddings",
            "input needed",
            "/delegate",
            "/rollback",
            "public apps",
            "access guard",
            "browser automation",
            "runtime access summary",
            "deployment topology",
            "connected systems",
            "workspace root",
            "default sandbox",
            "action permissions",
            "approval grants",
            "sandbox mode",
            "data contract",
            "data ownership",
            "user-owned",
            "system-owned",
        ],
    ) || contains_any_help_token(
        message,
        &[
            "sentinel", "arkpulse", "moltbook", "watchers", "swarm", "mcp", "executor",
        ],
    );

    help_intent && (high_signal_topic || explicit_product_signal)
}

#[cfg(test)]
pub fn infer_help_mode(message: &str) -> ProductHelpMode {
    let _ = message;
    ProductHelpMode::Explain
}

#[cfg(test)]
pub fn infer_help_topics(message: &str) -> Vec<&'static str> {
    let mut topics = Vec::new();
    let explicit_environment_phrase = contains_any_help_phrase(
        message,
        &[
            "where is it deployed",
            "where is this deployed",
            "what permissions does it have",
            "what can it access",
            "what is connected",
            "connected systems",
            "runtime access summary",
            "deployment topology",
            "investigate this instance",
            "workspace root",
            "default sandbox",
            "sandbox mode",
            "docker socket",
        ],
    );
    let environment_instance_signal = explicit_environment_phrase
        || contains_any_help_phrase(
            message,
            &[
                crate::branding::PRODUCT_NAME,
                "agentark",
                "agent ark",
                "this instance",
            ],
        );
    let environment_token_match = contains_any_help_token(
        message,
        &[
            "environment",
            "deployment",
            "deployed",
            "runtime",
            "sandbox",
            "docker",
            "executor",
            "workspace",
            "permissions",
            "approval",
            "approvals",
            "cpu",
            "cpus",
            "memory",
            "ram",
        ],
    );

    if contains_any_help_token(message, &["gmail"]) {
        topics.push("gmail");
    }
    if explicit_environment_phrase || (environment_token_match && environment_instance_signal) {
        topics.push("environment");
    }
    if topic_matches(
        message,
        &[
            "google workspace",
            "google cloud",
            "admin sdk",
            "google chat",
        ],
        &["gws", "oauth", "calendar", "drive", "docs", "sheets"],
    ) {
        topics.push("google_workspace");
    }
    if contains_any_help_token(message, &["moltbook"]) {
        topics.push("moltbook");
    }
    if topic_matches(
        message,
        &[
            "self learning",
            "self-learning",
            "arkevolve",
            "ark evolve",
            "background learning",
            "heuristic reflection",
            "heuristics",
            "experiential reflective learning",
            "erl",
            "reflection pass",
            "experience consolidation",
            "pattern induction",
            "candidate generation",
            "replay gate",
            "learning candidate",
            "learned memory",
            "learned procedure",
        ],
        &["evolution", "canary"],
    ) {
        topics.push("self_learning");
    }
    if topic_matches(
        message,
        &[
            "sentinel",
            "arksentinel",
            "ark sentinel",
            "background learning",
            "ambient engine",
        ],
        &[],
    ) {
        topics.push("sentinel");
    }
    if topic_matches(
        message,
        &["first run", "build from source"],
        &["install", "start", "docker"],
    ) {
        topics.push("install");
    }
    if topic_matches(message, &["new user", "i am new", "i'm new"], &[]) {
        topics.push("new_user");
    }
    if topic_matches(
        message,
        &["what can", "what does"],
        &["capabilities", "features"],
    ) {
        topics.push("capabilities");
    }
    if topic_matches(
        message,
        &["mission control", "approval inbox"],
        &["chat", "inbox"],
    ) {
        topics.push("chat");
    }
    if topic_matches(
        message,
        &["where do i", "where is", "navigation"],
        &["settings"],
    ) {
        topics.push("settings");
    }
    if contains_any_help_token(message, &["models", "provider", "llm"]) {
        topics.push("models");
    }
    if topic_matches(
        message,
        &[
            "embedding provider",
            "local embeddings",
            "external embeddings",
            "semantic search",
        ],
        &["embedding", "embeddings", "retrieval", "vector"],
    ) {
        topics.push("embeddings");
    }
    if contains_any_help_token(
        message,
        &[
            "media",
            "image",
            "video",
            "dall-e",
            "gemini",
            "veo",
            "replicate",
            "runway",
            "luma",
        ],
    ) {
        topics.push("media");
    }
    if topic_matches(
        message,
        &["daily brief"],
        &[
            "telegram", "slack", "discord", "matrix", "teams", "whatsapp",
        ],
    ) {
        topics.push("channels");
    }
    if contains_any_help_token(
        message,
        &["memory", "knowledge", "facts", "preferences", "user data"],
    ) {
        topics.push("memory");
    }
    if topic_matches(
        message,
        &[
            "data contract",
            "data ownership",
            "user-owned",
            "system-owned",
            "release updates",
            "settings kv",
            "settings:*",
        ],
        &["persistence", "persisted", "upgrade", "upgrades", "volumes"],
    ) {
        topics.push("data_contract");
    }
    if contains_any_help_token(
        message,
        &[
            "document",
            "documents",
            "upload",
            "file",
            "files",
            "library",
        ],
    ) {
        topics.push("documents");
    }
    if contains_any_help_token(message, &["watcher", "watchers", "monitor"]) {
        topics.push("watchers");
    }
    if contains_any_help_token(message, &["task", "tasks", "schedule"]) {
        topics.push("tasks");
    }
    if topic_matches(
        message,
        &[
            "input needed",
            "waiting on you",
            "missing input",
            "missing inputs",
        ],
        &[],
    ) {
        topics.push("input_needed");
        topics.push("tasks");
    }
    if contains_any_help_token(message, &["app", "apps", "deploy"]) {
        topics.push("apps");
    }
    if contains_any_help_token(
        message,
        &[
            "channel", "telegram", "whatsapp", "slack", "discord", "teams",
        ],
    ) {
        topics.push("channels");
    }
    if topic_matches(
        message,
        &[
            "custom integration",
            "custom integrations",
            "user added integration",
            "user-added integration",
            "extension pack",
            "extension packs",
            "pack based integration",
            "pack-based integration",
        ],
        &[],
    ) {
        topics.push("custom_integrations");
        topics.push("integrations");
    }
    if contains_any_help_token(message, &["integrations", "integration", "connectors"]) {
        topics.push("integrations");
    }
    if contains_any_help_phrase(message, &["plugin sdk"])
        || contains_any_help_token(message, &["plugin", "plugins"])
    {
        topics.push("plugins");
    }
    if topic_matches(
        message,
        &["custom api", "custom apis", "incoming webhook"],
        &["webhook", "webhooks"],
    ) {
        topics.push("webhooks");
    }
    if topic_matches(
        message,
        &["skill import"],
        &["skills", "skill", "capability"],
    ) {
        topics.push("skills");
    }
    if topic_matches(
        message,
        &[
            "chat shortcuts",
            "/notifications pause",
            "/notifications resume",
            "/notifications status",
            "/delegate",
            "/rollback",
        ],
        &[],
    ) {
        topics.push("chat_shortcuts");
    }
    if topic_matches(
        message,
        &["specialist agent", "specialist agents", "agents page"],
        &["swarm"],
    ) {
        topics.push("swarm");
    }
    if contains_any_help_token(message, &["goal", "goals"]) {
        topics.push("goals");
    }
    if topic_matches(
        message,
        &["execution trace", "what did it do"],
        &["trace", "logs"],
    ) {
        topics.push("trace");
    }
    if topic_matches(
        message,
        &["health check", "operational pulse"],
        &["arkpulse", "pulse"],
    ) {
        topics.push("arkpulse");
    }
    if topic_matches(message, &["usage metrics", "llm analytics"], &["analytics"]) {
        topics.push("analytics");
    }
    if topic_matches(
        message,
        &["website", "form fill", "browser automation"],
        &["browser"],
    ) {
        topics.push("browser");
    }
    if topic_matches(
        message,
        &["web search", "deep research", "search the web"],
        &["research"],
    ) {
        topics.push("research");
    }
    if topic_matches(
        message,
        &["api key"],
        &["security", "advanced", "mcp", "observability", "webhook"],
    ) {
        topics.push("security");
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
    BUNDLED_HELP_DOCS
        .iter()
        .map(|doc| SeedKnowledgeItem {
            title: branded_product_text(doc.title),
            content: branded_product_text(&render_bundled_help_doc(doc)),
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
        ("Skills", "/skills", "Reusable skills and imports."),
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
        ("Moltbook", "/moltbook", "Top-level Moltbook control page."),
        (
            "Tasks",
            "/tasks",
            "Durable queue, schedules, and approvals.",
        ),
        (
            "ArkSentinel",
            "/sentinel",
            "ArkSentinel view with proposals, observations, and Background learning status.",
        ),
        (
            "Watchers",
            "/watchers",
            "Background poll-until-condition monitors.",
        ),
        ("ArkPulse", "/arkpulse", "Operational pulse and guidance."),
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
                "Settings > Integrations > Custom Integrations",
                "Settings > Integrations > Webhooks & APIs",
                "Settings > Integrations > Plugins",
            ],
        ),
        (
            "Knowledge",
            &[
                "ArkMemory",
                "Settings > Knowledge > MCP Servers",
            ],
        ),
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
- Use the request-scoped runtime access summary and action catalog as the live source of truth for what this instance can do right now.\n\
- Use integration inventory for connected channels and connectors.\n\
- Use MCP Servers and Plugins settings when the user asks about external capability extensions.\n\
- Use Tasks, Watchers, Goals, Apps, Trace, Analytics, and ArkPulse to inspect durable work and operational state.\n\
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
- Library > Documents | Uploaded files and indexed document context.\n\
- ArkMemory | Structured memory, source attribution, review, rollback, and reusable knowledge-base items.\n\
- ArkMemory > Current Memory > Facts | Learned facts and operating constraints captured by the memory system.\n\
- ArkMemory > Current Memory > Preferences | Durable user preferences and rules.\n\
- ArkMemory > Current Memory > User Data | Notes, links, and captured user data.\n\
- ArkMemory > Current Memory > Knowledge | Reusable knowledge-base items, including bundled product docs after sync.\n\
- Settings > Knowledge > MCP Servers | External MCP server configuration.";
    items.push(SeedKnowledgeItem {
        title: "Library, documents, and memory surfaces".to_string(),
        content: branded_product_text(library_content),
        source: RUNTIME_SOURCE,
        url: Some(crate::branding::help_uri("help/runtime/library-memory")),
        tags: Some(
            "library, documents, memory, knowledge, files, uploads, facts, preferences, user_data, mcp"
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

    let evolution_content = "ArkEvolve and self-learning surfaces in the current UI.\n\n\
- ArkEvolve | Main self-learning page with What happened, What helped, Tests running, Review, and developer controls.\n\
- ArkSentinel > Background learning | Live status for heuristic reflection, experience consolidation, pattern induction, and candidate generation.\n\
- Learned Heuristics | Short transferable lessons distilled from completed runs.\n\
- ArkEvolve > What happened | Recent tested or promoted changes with lineage and plain-language summaries.\n\
- ArkEvolve > What helped | Measured impact from prompt, classifier, specialist, and routing changes.\n\
- ArkEvolve > Tests running | Canary rollout, baseline version, candidate version, and gate result for each evolvable surface.\n\
- ArkEvolve > Review | Draft workflow, strategy, and memory candidates waiting for review.\n\
- Settings > Advanced > ArkSentinel | Keep ArkSentinel available, choose whether it watches AgentArk activity or connected apps, and control routine detection.\n\
- Settings > Advanced > ArkEvolve | Self-evolve master switch for background learning and canary experiments.\n\
- Settings > Advanced > App Deploy Defaults | Default app access guard for new app deploy and public-link flows.\n\
- ArkEvolve > Controls | Developer-mode canary actions and manual testing controls.\n\
- Learned Memory | Durable facts, rules, lessons, and memory extracted from runs.\n\
- Learned Procedures | Repeated successful workflows distilled into procedures.\n\
- Recent Experience Runs | Recent evidence feeding the learning system.\n\
- Learning Candidates | Draft workflow/strategy/memory actions waiting for review.\n\
- Canary History and Strategy Metrics | Diagnostics for policy rollout and promotion decisions.";
    items.push(SeedKnowledgeItem {
        title: "ArkEvolve and self-learning surfaces".to_string(),
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
            "Live action snapshot. If an action appears here, this {} instance can use it when the request matches and required credentials/config are present.\n\n",
            crate::branding::PRODUCT_NAME,
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
            url: Some(crate::branding::help_uri(&format!(
                "help/runtime/actions-{}",
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
    fn detects_agentark_help_queries() {
        assert!(looks_like_agentark_help_query(&format!(
            "How do I add Gmail access in {}?",
            crate::branding::PRODUCT_NAME
        )));
        assert!(looks_like_agentark_help_query(
            "I am new, how do I run Moltbook?"
        ));
        assert!(!looks_like_agentark_help_query(
            "Post on Moltbook about this release"
        ));
        assert!(looks_like_agentark_help_query(&format!(
            "How do I use {}?",
            crate::branding::PRODUCT_NAME
        )));
        assert!(!looks_like_agentark_help_query(
            "How do I debug Python tasks?"
        ));
    }

    #[test]
    fn infers_help_topics() {
        let topics = infer_help_topics("How do I add Gmail access with Google Workspace?");
        assert!(topics.contains(&"gmail"));
        assert!(topics.contains(&"google_workspace"));
    }

    #[test]
    fn matches_capabilities_overview_for_general_product_queries() {
        let matches = match_bundled_help_docs("What is AgentArk?", 3);
        assert!(!matches.is_empty());
        assert_eq!(matches[0].slug, "capabilities-overview");
        assert!(matches[0].tags.iter().any(|tag| tag == "general"));
    }

    #[test]
    fn does_not_force_agentark_overview_for_generic_agent_queries() {
        let matches = match_bundled_help_docs("What is an agent?", 3);
        assert!(matches.is_empty());
    }

    #[test]
    fn derives_help_topics_from_matched_docs() {
        let matches = match_bundled_help_docs("How do I configure models in AgentArk?", 3);
        let topics = infer_help_topics_from_bundled_matches(&matches);
        assert!(topics.iter().any(|topic| topic == "models"));
    }

    #[test]
    fn recognizes_google_calendar_help_queries() {
        assert!(looks_like_agentark_help_query(
            "How do I connect Google Calendar?"
        ));
        let topics = infer_help_topics("How do I connect Google Calendar?");
        assert!(topics.contains(&"google_workspace"));
    }

    #[test]
    fn recognizes_google_chat_help_queries_without_broadening_to_chatgpt() {
        assert!(looks_like_agentark_help_query(
            "How do I set up Google Chat?"
        ));
        let topics = infer_help_topics("How do I set up Google Chat?");
        assert!(topics.contains(&"google_workspace"));
        assert!(!infer_help_topics("What is ChatGPT?").contains(&"google_workspace"));
    }

    #[test]
    fn detects_self_learning_help_queries() {
        assert!(looks_like_agentark_help_query(&format!(
            "How does self-learning work in {}?",
            crate::branding::PRODUCT_NAME
        )));
        assert!(
            infer_help_topics("What has ArkEvolve learned recently?").contains(&"self_learning")
        );
        assert!(matches!(
            infer_help_mode("Is self-learning enabled right now?"),
            ProductHelpMode::Explain
        ));
    }

    #[test]
    fn detects_background_learning_help_queries() {
        assert!(looks_like_agentark_help_query(
            "Check background learning status in ArkSentinel"
        ));
        let topics = infer_help_topics("Why is background learning in ArkSentinel not running?");
        assert!(topics.contains(&"self_learning"));
        assert!(topics.contains(&"sentinel"));
        assert!(matches!(
            infer_help_mode("What is the current background learning status?"),
            ProductHelpMode::Explain
        ));
    }

    #[test]
    fn detects_embeddings_help_queries() {
        assert!(looks_like_agentark_help_query(
            "Where do I change the embedding provider in AgentArk?"
        ));
        let topics = infer_help_topics("How do local embeddings work?");
        assert!(topics.contains(&"embeddings"));
    }

    #[test]
    fn detects_environment_help_queries() {
        assert!(looks_like_agentark_help_query(
            "What environment is AgentArk running in right now?"
        ));
        let topics = infer_help_topics(
            "Where is this AgentArk instance deployed, what CPUs are visible, and what permissions does it have?",
        );
        assert!(topics.contains(&"environment"));
        assert!(matches!(
            infer_help_mode("What environment is AgentArk running in right now?"),
            ProductHelpMode::Explain
        ));
    }

    #[test]
    fn detects_input_needed_help_queries() {
        assert!(looks_like_agentark_help_query(
            "What does Input needed mean?"
        ));
        let topics = infer_help_topics("Why is this task waiting on you with Input needed?");
        assert!(topics.contains(&"input_needed"));
        assert!(topics.contains(&"tasks"));
    }

    #[test]
    fn detects_chat_shortcuts_help_queries() {
        assert!(looks_like_agentark_help_query(
            "How do I use /rollback in AgentArk?"
        ));
        let topics = infer_help_topics("What does /rollback do?");
        assert!(topics.contains(&"chat_shortcuts"));
    }

    #[test]
    fn detects_custom_integration_help_queries() {
        assert!(looks_like_agentark_help_query(
            "How do I install a custom integration in AgentArk?"
        ));
        let topics = infer_help_topics("How do I add a user-added extension pack?");
        assert!(topics.contains(&"custom_integrations"));
        assert!(topics.contains(&"integrations"));
    }

    #[test]
    fn matches_custom_integrations_doc_for_custom_setup_queries() {
        let matches = match_bundled_help_docs(
            "How do I add Linear as a custom integration in AgentArk?",
            3,
        );
        assert!(!matches.is_empty());
        assert_eq!(matches[0].slug, "custom-integrations-and-extension-packs");
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
            .any(|item| item.title == "Plugins, webhooks, and custom APIs"));
        assert!(items
            .iter()
            .any(|item| item.title == "Runtime environment and investigation"));
    }
}
