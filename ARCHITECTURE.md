# AgentArk Architecture

Design principles, language rationale, project structure, and a documentation map for navigating the codebase. For installation and feature overviews, see the [README](README.md).

## Design Principles

- **Secure first** - encrypted secrets, approvals, sandboxing, and verifiable records
- **Daily by default** - briefs, reminders, follow-up, and messaging delivery are first-class
- **Memory that compounds** - Memory builds on previous preferences, facts, sources, and reviewed memory changes
- **Self-evolving** - corrections, tool outcomes, and benchmarks improve local memory, lessons, procedures, prompts, classifiers, specialist prompts, routing, and strategy; skills remain separately designed or installed capabilities
- **Chat-first** - talk to it naturally, not through config files or flowcharts
- **Power when needed** - tasks, watchers, apps, integrations, and swarm agents for deeper work
- **Model-agnostic** - OpenAI, Anthropic, Google, Ollama, or any OpenAI-compatible endpoint
- **Self-hosted** - your hardware, your data, your rules

## Why Rust?

|                     |                                                                                   |
| :------------------ | :-------------------------------------------------------------------------------- |
| **Performance**     | Tokio async runtime, `Arc<RwLock<T>>` concurrency - no GIL bottleneck             |
| **Security**        | `Zeroizing` auto-clears secrets from memory; zero `unsafe` blocks in the codebase |
| **Type Safety**     | Enums, traits, and compile-time guarantees catch bugs before production           |
| **Single Binary**   | One compiled binary + Docker - no dependency hell                                 |
| **WASM Sandboxing** | Wasmtime integration is natural in Rust                                           |

## Project structure

The backend is organized into domain-grouped module subtrees. The high-level layout:

```text
src/
├── core/
│   ├── agent/          # Spine turn loop, conversation, memory, skills, runtime
│   ├── orchestration/  # Multi-agent coordination
│   ├── swarm/          # Specialist agents, coordination, shared state
│   ├── knowledge/      # Learning, memory dedup, embeddings
│   ├── model/          # LLM routing and providers
│   ├── runtime/        # Config, secrets, operations
│   ├── platform/       # Observability and platform services
│   ├── connectivity/   # Integrations and channel plumbing
│   ├── automation/     # Autonomy and scheduled work
│   └── self_evolve/    # Gates, optimizer, policies, prompts, routing
├── actions/            # Tool implementations (app, deploy, google, network, research)
├── channels/           # Messaging, gateway, outbound, web, and the HTTP API
│   └── http/           # core_api, admin_api, analytics_api, app_api,
│                       #   automation_api, integration_api, runtime_api, tunnel_api
├── security/           # guards, boundary, classification, privacy, review, model, abuse
├── runtime/            # WASM + Docker sandboxing, action runtime
├── storage/            # Postgres persistence, entities, repositories
├── integrations/       # Grouped: google, messaging, browser, productivity, media, ...
├── sentinel/           # Follow-up scanning and health findings
├── telemetry/          # Metrics and tracing
└── main.rs             # Entrypoint

frontend/src/           # React + MUI + TypeScript web UI
skills/                 # Built-in skill definitions
```

For the contributor-facing quick map and development setup, see [CONTRIBUTING.md](CONTRIBUTING.md).

## Documentation Map

For documentation generators such as DeepWiki, these are the main product concepts and source areas to index first.

| Area                   | Start here                                                                                                                                          | Notes                                                                                                                                         |
| :--------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------- |
| Product shell          | `frontend/src/App.tsx`, `frontend/src/components/NativeWorkspace.tsx`, `frontend/src/styles.css`                                                     | Navigation, responsive shell, Mission Control, Chat, Memory, Reflect, Sentinel, Evolve, Pulse, and settings surfaces           |
| API surface            | `src/channels/http/mod.rs`, `src/channels/http/*`                                                                                                   | HTTP routes, settings, integrations, companion devices, model control, webhooks, Reflect, Pulse, Sentinel, and Memory panels      |
| Agent runtime          | `src/core/agent/mod.rs`, `src/core/agent/*`, `src/runtime/mod.rs`                                                                                   | Tool planning, execution loop, approvals, sandboxing, task routing, generated apps, action traces, and response delivery                      |
| Memory and learning    | `src/core/knowledge/learning.rs`, `src/core/knowledge/memory_dedup.rs`, `src/storage/entities/experience_item.rs`                                   | User facts, preferences, Memory views, semantic deduplication, provenance, review, rollback, and consolidation                             |
| Reflect             | `src/channels/http/automation_api/reflect_control.rs`, `src/storage/entities/semantic_work_unit.rs`, `frontend/src/components/pages/ArkReflectPage.tsx` | Cached local retrospectives, derived semantic work units, day/week/month clustering, source coverage, related-history lookup, and Panorama UI |
| Sentinel            | `src/sentinel/mod.rs`, `src/channels/http/automation_api/sentinel_panel.rs`, `src/core/automation/autonomy.rs`                                      | Follow-up scanning, routine detection, health findings, proposals, scheduled work, and automation nudges                                      |
| Evolve              | `src/core/self_evolve/*`, `src/core/agent/runtime/tool_execution.rs`                                                                                | Prompt, policy, classifier, and specialist evolution with canaries, replay evaluation, promotion gates, and rollback                          |
| Pulse               | `src/sentinel/mod.rs`, `src/core/platform/observability.rs`, `src/core/runtime/operations/release_updates.rs`                                       | Runtime health checks, remediation hints, operational findings, update status, and system readiness surfaces                                  |
| Integrations and packs | `src/extension_packs/mod.rs`, `src/channels/http/integration_api/integrations.rs`, `frontend/src/components/IntegrationsPanel.tsx`                  | Extension packs, messaging channels, OAuth/setup wizards, custom APIs, MCP, webhooks, install/delete cleanup, and secrets handling            |
| Companion devices      | `src/core/connectivity/channels/companion.rs`, `src/channels/http/app_api/companion_control.rs`, `frontend/src/components/CompanionDevicesPanel.tsx` | Pairing, scoped grants, high-risk approvals, audit trail, device commands, and queued actions                                                 |
| Storage and secrets    | `src/storage/*`, `src/core/runtime/config/config.rs`, `src/core/runtime/config/secrets.rs`, `src/storage/encrypted.rs`                              | Postgres entities, schema setup, encrypted config, secret storage, retention, cleanup, and audit data                                         |

Key flows worth documenting:

- Chat request -> plan/tool loop -> trace -> response -> memory and automation updates.
- Memory capture -> semantic deduplication -> review and provenance -> rollback when needed.
- Background session, task, or watcher -> Sentinel follow-up -> approval or scheduled action.
- Reflect refresh -> bounded source scan -> derived semantic work units -> cached clusters and visual recap.
- Integration install or delete -> config, secrets, files, and audit cleanup.
- App generation and deployment -> sandbox/runtime -> private or public access -> Pulse health checks.
- Evolve review candidate -> past-example test -> approval or rejection -> apply or leave unchanged.
- Evolve prompt/policy candidate -> benchmark -> limited live rollout or promotion -> stop, disable, or rollback where supported.
