#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AgentDocTocEntry {
    pub label: &'static str,
    pub location: &'static str,
    pub use_for: &'static str,
}

pub(crate) const AGENT_DOC_TOC: &[AgentDocTocEntry] = &[
    AgentDocTocEntry {
        label: "Bundled product help",
        location: "src/docs/product_help.rs",
        use_for: "user-facing setup, navigation, feature, Docker, memory, and integration guidance",
    },
    AgentDocTocEntry {
        label: "Product help retrieval",
        location: "src/core/product_help.rs",
        use_for: "how bundled help docs become searchable knowledge and runtime help context",
    },
    AgentDocTocEntry {
        label: "Primary system prompt",
        location: "src/core/agent/prompt_builder.rs",
        use_for: "runtime prompt assembly, action-catalog context, identity, and response policy",
    },
    AgentDocTocEntry {
        label: "Agent request loop",
        location: "src/core/agent.rs",
        use_for: "chat handling, memory capture, context injection, routing, runtime state, and execution flow",
    },
    AgentDocTocEntry {
        label: "Actions and schemas",
        location: "src/actions/",
        use_for: "available tools, action definitions, schemas, permission behavior, and connector-backed capabilities",
    },
    AgentDocTocEntry {
        label: "HTTP API and web UI",
        location: "src/channels/http.rs; frontend/src/components/NativeWorkspace.tsx",
        use_for: "API routes, settings endpoints, local web UI behavior, and browser-visible workflows",
    },
    AgentDocTocEntry {
        label: "Memory and storage",
        location: "src/memory/; src/storage/",
        use_for: "semantic facts, preferences, encrypted persistence, Postgres schema, and database access",
    },
    AgentDocTocEntry {
        label: "Learning and evolution",
        location: "src/core/learning.rs; src/core/self_evolve/",
        use_for: "experience items, prompt evolution, background learning, and self-improvement workflows",
    },
    AgentDocTocEntry {
        label: "Security and secrets",
        location: "SECURITY.md; src/security/; src/crypto/",
        use_for: "secret handling, encryption, API-token safety, approvals, and security expectations",
    },
    AgentDocTocEntry {
        label: "Local run and Docker",
        location: "README.md; Dockerfile; docker-compose.yml; docker-compose.dev.yml",
        use_for: "local startup, rebuilds, volume reset behavior, and container layout",
    },
    AgentDocTocEntry {
        label: "Verification and contribution",
        location: "VERIFY.md; CONTRIBUTING.md; .github/workflows/",
        use_for: "test commands, CI expectations, release checks, and contributor workflow",
    },
    AgentDocTocEntry {
        label: "Bundled skills",
        location: "skills/; src/docs/product_help.rs",
        use_for: "built-in skill docs, skill discovery, and product help that references bundled skills",
    },
];

pub(crate) fn render_agent_doc_toc() -> String {
    use std::fmt::Write as _;

    let mut out = String::from(
        "## Agent Documentation Map\n\
         - Start here as a table of contents for AgentArk's local knowledge.\n\
         - This map is not the full documentation. When a task needs implementation details, inspect the referenced source or doc path first instead of relying on this summary.\n\
         - Use runtime inspection for current state such as containers, settings, tasks, traces, apps, and integrations.\n",
    );

    for entry in AGENT_DOC_TOC {
        let _ = writeln!(
            out,
            "- {}: `{}` - {}.",
            entry.label, entry.location, entry.use_for
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_doc_toc_renders_compact_routing_map() {
        let rendered = render_agent_doc_toc();

        assert!(rendered.contains("table of contents"));
        assert!(rendered.contains("src/docs/product_help.rs"));
        assert!(rendered.contains("src/core/agent/prompt_builder.rs"));
        assert!(rendered.contains("src/storage/"));
        assert!(rendered.lines().count() <= 20);
    }
}
