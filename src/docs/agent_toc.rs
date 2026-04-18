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
        use_for: "user-facing setup, navigation, feature, Docker, memory, built-in connector, and custom integration guidance",
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
        label: "Companion devices",
        location: "src/core/companion.rs; src/channels/http/companion_control.rs; frontend/src/components/CompanionDevicesPanel.tsx; clients/companion/; src/docs/product_help.rs",
        use_for: "paired device setup, scoped grants, WebSocket protocol, typed companion commands, approval requirements, first-party native clients, and custom-device guidance",
    },
    AgentDocTocEntry {
        label: "HTTP API and web UI",
        location: "src/channels/http.rs; frontend/src/components/NativeWorkspace.tsx",
        use_for: "API routes, settings endpoints, local web UI behavior, and browser-visible workflows",
    },
    AgentDocTocEntry {
        label: "Custom messaging channels",
        location: "src/custom_messaging_channels/mod.rs; src/channels/messaging_registry.rs; src/channels/messaging_dispatch.rs; frontend/src/components/IntegrationsPanel.tsx",
        use_for: "user-added outbound notification channels, secure credential forms, registry gating, and HTTP dispatch templates",
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
        label: "Prompt telemetry and canary safety",
        location: "src/core/llm.rs; src/core/agent.rs; src/core/learning.rs; src/core/observability.rs; src/channels/http.rs; frontend/src/components/NativeWorkspace.tsx",
        use_for: "final prompt and tool-schema telemetry, Trace prompt-telemetry steps, ArkEvolve review signals, observability export, and prompt-profile canary safety review flows",
    },
    AgentDocTocEntry {
        label: "Security and secrets",
        location: "SECURITY.md; src/security/; src/security/capabilities.rs; src/security/skill_review.rs; src/crypto/; src/docs/product_help.rs",
        use_for: "secret handling, encryption, API-token safety, approvals, inbound guard behavior, security alerts, and security expectations",
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
        label: "Skill management",
        location: "src/docs/product_help.rs; src/security/skill_review.rs; src/security/capabilities.rs; src/channels/http/actions.rs; src/runtime/mod.rs",
        use_for: "user-added skill import, editing, semantic capability review, deterministic skill policy, and reviewed skill runtime loading",
    },
];

pub(crate) fn render_agent_doc_toc() -> String {
    use std::fmt::Write as _;

    let mut out = String::from(
        "## Agent Documentation Map\n\
         - Start here as a table of contents for AgentArk's local personal AI OS knowledge.\n\
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
