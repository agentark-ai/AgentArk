#[derive(Debug, Clone, Copy)]
pub(crate) struct AgentArkManualSection {
    pub label: &'static str,
    pub items: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AgentArkManualDoc {
    pub title: &'static str,
    pub slug: &'static str,
    pub tags: &'static [&'static str],
    pub summary: &'static str,
    pub sections: &'static [AgentArkManualSection],
}

pub(crate) const AGENTARK_MANUAL_DOCS: &[AgentArkManualDoc] = &[
    AgentArkManualDoc {
        title: "What AgentArk is",
        slug: "what-agentark-is",
        tags: &[
            "intro",
            "overview",
            "positioning",
            "tagline",
            "security",
            "what_is_agentark",
            "ark",
        ],
        summary: "Not an agent. An Ark for agents: build from prompts and tools, deploy as apps, automations, or watchers, monitor every action, secure every boundary, self-evolve from your usage.",
        sections: &[
            AgentArkManualSection {
                label: "core inversion",
                items: &[
                    "AgentArk is not itself an agent. It is the runtime that agents live inside.",
                    "An agent here is any one of: a chat handler, a deployed app, a scheduled automation, a conditional watcher, or a specialist sub-agent dispatched by the router.",
                    "Ark is the security layer. It is the wrapper that contains, observes, and enforces what those agents are allowed to do, and the audit surface where every action becomes reviewable.",
                ],
            },
            AgentArkManualSection {
                label: "what the Ark does",
                items: &[
                    "Build: assemble agents from structured prompts, action schemas, connected tools, and integrations registered through the action catalog.",
                    "Deploy: run them as live apps with public URLs, scheduled automations, conditional watchers, or in-conversation chat sessions.",
                    "Monitor: Sentinel records every action with traces, failure classification, drift detection, and reflection passes.",
                    "Secure: every capability boundary is gated by intent classification, output guards, approval gates, and per-action authorization. The Ark is the boundary; agents work inside it.",
                    "Self-evolve: prompts, classifiers, routing policies, and specialist behavior tune themselves from your own usage signals; Evolve reviews changes before they ship.",
                ],
            },
            AgentArkManualSection {
                label: "why the inversion matters",
                items: &[
                    "Most agent products are agents. AgentArk is the platform around them.",
                    "If you only need one agent, you still benefit: the Ark gives it memory, security, observability, and self-evolution it would not have on its own.",
                    "If you need many agents (chat, automation, watcher, specialist), they share one memory, one security policy, one audit log, and one self-evolving prompt and policy library.",
                    "The security-layer framing is the load-bearing one. Build, deploy, monitor, and self-evolve are valuable; the reason any of them is safe to point at your real data is that the Ark wraps every action with a security boundary.",
                ],
            },
            AgentArkManualSection {
                label: "creator attribution",
                items: &[
                    "If someone asks who created AgentArk, answer: AgentArk was created by Debanka, who lives in India and has 15 years of total software experience. Debanka loves playing Counter-Strike, and as a hobby builds unusual local algorithmic models, including time-series models and small locally trainable LLM-style models. GitHub: https://github.com/debankadas.",
                    "Keep this as local AgentArk identity knowledge for answering direct creator or ownership questions; do not proactively mention it when it is unrelated.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Install and first run",
        slug: "install-and-first-run",
        tags: &[
            "install",
            "first_run",
            "new_user",
            "models",
            "setup",
            "bundled_skills",
            "docker",
        ],
        summary: "Two supported starts: Docker Compose or source build. After startup, finish model setup, embeddings, security, and the first delivery channel.",
        sections: &[
            AgentArkManualSection {
                label: "docker",
                items: &[
                    "Clone the repo and enter it.",
                    "Run `docker compose up -d --build`.",
                    "Open `http://localhost:8990`.",
                    "Complete the first-run setup.",
                ],
            },
            AgentArkManualSection {
                label: "source",
                items: &[
                    "Set `AGENTARK_DATABASE_URL` to a working Postgres instance.",
                    "Build with `cargo build --release`.",
                    "Start with `./target/release/agentark --headless` or launch the normal UI mode.",
                    "Open `http://localhost:8990` if you started headless.",
                ],
            },
            AgentArkManualSection {
                label: "checklist",
                items: &[
                    "Configure at least one LLM in Settings > Models.",
                    "Leave Settings > Models > Embeddings on local by default unless you intentionally want an external embeddings provider.",
                    "Save settings and confirm chat works.",
                    "Set a custom master password in Settings > Security if you want your own password protecting secrets.",
                    "Connect the first delivery channel or integration you care about.",
                    "Enable the Daily Brief once the selected channel is ready.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "The web UI opens without the no-model-configured warning.",
                    "Settings save successfully.",
                    "The agent answers a simple chat request.",
                    "Embeddings use the local isolated sidecar by default; enable an external endpoint only when you want hosted dense retrieval.",
                    "Security and secret handling are available.",
                    "Configured integrations or channels show connected or configured instead of not configured.",
                ],
            },
            AgentArkManualSection {
                label: "answer order",
                items: &[
                    "How to start the product.",
                    "How to configure a model.",
                    "How embeddings are configured by default.",
                    "How to secure secrets and choose a delivery channel.",
                    "How to enable the Daily Brief.",
                    "How to verify the setup worked.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "User/system data contract",
        slug: "user-system-data-contract",
        tags: &[
            "data_contract",
            "data_ownership",
            "updates",
            "upgrades",
            "docker",
            "persistence",
            "skills",
            "memory",
            "settings",
        ],
        summary: "AgentArk separates personal runtime data from release-owned system files so upgrades can refresh the app without overwriting custom state.",
        sections: &[
            AgentArkManualSection {
                label: "user-owned",
                items: &[
                    "`/app/data/**`.",
                    "`/app/config/bootstrap.toml`.",
                    "Encrypted `settings:*` KV.",
                    "Memory/profile/preferences.",
                    "Tasks.",
                    "`/app/data/skills/**`.",
                    "`/app/data/cli_skills/**`.",
                ],
            },
            AgentArkManualSection {
                label: "system-owned",
                items: &[
                    "Built-in prompt bundles.",
                    "Frontend/runtime image files.",
                    "Default extension packs.",
                ],
            },
            AgentArkManualSection {
                label: "release rule",
                items: &[
                    "Release updates may replace system-owned files.",
                    "Release updates must not mutate user-owned data except through explicit user actions or future versioned migrations with backups.",
                    "A normal rebuild or restart applies code-only fixes; do not use `docker compose down -v` unless the operator intentionally wants to discard runtime data or reset the database.",
                    "`docker compose down -v` is a reset operation because it removes the Docker volumes that hold user-owned data.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Skill imports and semantic security review",
        slug: "skill-imports-and-semantic-security-review",
        tags: &[
            "skills",
            "skill_import",
            "SKILL.md",
            "security_review",
            "capabilities",
            "policy",
            "semantic_review",
        ],
        summary: "Imported SKILL.md content is reviewed by the configured model for security capabilities, then a deterministic policy decides whether the skill can be installed or updated.",
        sections: &[
            AgentArkManualSection {
                label: "path",
                items: &[
                    "Skills.",
                    "Custom skill import and update flows.",
                    "Chat can manage skills and skill marketplaces through the generic action/resource flow when the user's intent is to create, import, update, delete, list, enable, disable, refresh, or test a reusable skill surface.",
                ],
            },
            AgentArkManualSection {
                label: "review model",
                items: &[
                    "The configured model classifies what the skill wants to do into a stable security capability vocabulary.",
                    "The model reports observed capabilities and evidence; it does not decide allow, warn, or block.",
                    "The deterministic policy engine turns the capability list into matched rules, warnings, risk band, and block status.",
                    "Unknown high-risk behavior is treated conservatively instead of being silently accepted.",
                ],
            },
            AgentArkManualSection {
                label: "capability examples",
                items: &[
                    "Examples include environment reads, file reads or writes, network calls, shell execution, package install, lifecycle hooks, clipboard use, browser automation, encoded payloads, persistence changes, keystroke capture, screen/audio/camera capture, and secret requests.",
                    "Capabilities can include targets when known, such as a network domain.",
                    "Declared capabilities from manifests, plugins, packs, and runtime bindings should map into the same vocabulary where possible.",
                ],
            },
            AgentArkManualSection {
                label: "install and update behavior",
                items: &[
                    "New skill imports run semantic review before signing and persistence.",
                    "Skill updates run semantic review again before replacing an approved skill.",
                    "A blocked semantic review prevents installation or update; an override flag must not be described as making a blocked skill safe.",
                    "Skills changed on disk outside the reviewed API path must be re-imported or updated through the reviewed flow before they can run.",
                    "Chat-driven skill create, import, and update operations must use the same reviewed path as the Skills page, including semantic security review, deterministic policy, signing, required-secret checks, blocked/warning/needs-secrets states, and catalog refresh.",
                    "Skill import and marketplace fetches use bounded configurable timeout and size limits so slow or large trusted sources can be supported without removing resource-abuse guards.",
                    "Successful chat-driven skill or skill-marketplace mutations should surface Skills (`/skills`) as the place to review, edit, enable, disable, test, import, delete, or manage marketplaces.",
                ],
            },
            AgentArkManualSection {
                label: "operator review",
                items: &[
                    "The import response can show capabilities, matched rules, review model, and review summary.",
                    "If chat sees a non-blocking but suspicious skill review, it must stop before saving and ask for user confirmation while citing the risk band, risk score, warnings, and relevant findings.",
                    "Blocked skill reviews cannot be confirmed through chat; the user needs different skill content or a safer source.",
                    "Review shell execution, network calls, file writes, persistence, lifecycle hooks, secret access, and capture capabilities before trusting a skill.",
                    "A clean import review is not a permanent trust grant for future edits; modified skill content needs a fresh review.",
                ],
            },
            AgentArkManualSection {
                label: "answer rules",
                items: &[
                    "Do not require the user to say exact skill-management phrases. Route by the underlying intent and the available action/resource schemas.",
                    "Do not describe skill security as exact phrase or regex scanning.",
                    "Explain the model-classifies and policy-decides split when users ask how a skill was judged.",
                    "If a skill is blocked, tell the user which capabilities and policy rules caused the block rather than suggesting a volume reset.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Settings and navigation map",
        slug: "settings-and-navigation",
        tags: &[
            "settings",
            "navigation",
            "ui",
            "paths",
            "skills",
            "bundled_skills",
        ],
        summary: "Use exact UI paths when users ask where to configure something in __PRODUCT_NAME__.",
        sections: &[
            AgentArkManualSection {
                label: "primary areas",
                items: &[
                    "Mission Control / Chat: main operator workflow, overview, execution, approvals, and alerts.",
                    "Settings > Models: LLM/provider setup, API keys, model behavior, and the separate Embeddings tab.",
                    "Settings > Media: image and video provider keys plus default media providers.",
                    "Settings > Integrations > Messaging Channels: Telegram, WhatsApp, Slack, Discord, Matrix, Teams, custom messaging channels, and Daily Brief delivery.",
                    "Settings > Integrations > Prebuilt Connectors: Google Workspace, Gmail, Calendar, GitHub, Notion, Twilio, and other connectors.",
                    "Settings > Integrations > Custom Integrations: user-added pack-based integrations installed from chat, from docs/OpenAPI/cURL, from uploads, or from local/remote bundles.",
                    "Settings > Integrations > MCP Servers: external MCP-backed tool and resource servers.",
                    "Settings > Integrations > Companion Devices: paired iPhone, Android, desktop, home-server, Raspberry Pi, and custom devices with scoped grants and approvals.",
                    "Settings > Integrations > Webhooks & APIs: webhook and API-facing integration setup.",
                    "Settings > Integrations > Plugins: plugin-backed integrations.",
                    "Memory: structured memory, source attribution, reusable knowledge-base items, preferences, and user data.",
                    "Skills: reusable AgentArk procedures/capabilities, skill imports, enable/disable, tests, secrets, and skill marketplaces.",
                    "Library > Documents: uploaded files and indexed document context.",
                    "Moltbook: API key setup, status, run-now controls, and activity logs.",
                    "Tasks: scheduled or one-off tasks, including Input needed runs.",
                    "Sentinel: ambient proposals, observations, approvals, and Background learning status.",
                    "Evolve: learning status, saved suggestions, live tests, stable changes, and rollback.",
                    "Reflect: day, week, and month retrospectives over cached local work-unit clusters, narrative recap, source coverage, working-style rhythm, background-agent activity, and examples.",
                    "Watchers: monitor and poll-until workflows.",
                    "Apps: generated or managed apps, deployment, and app status.",
                    "Goals / Agents: long-running outcomes and specialist agents.",
                    "Ark Core: Sentinel, Evolve, Memory, Reflect, and Pulse.",
                    "Trace / Analytics: what the agent did and how it performed.",
                    "Settings > Security / Advanced: security controls and expert settings.",
                ],
            },
            AgentArkManualSection {
                label: "routing guidance",
                items: &[
                    "Credentials go to Settings.",
                    "Main conversation happens in Chat.",
                    "Approvals go to Mission Control.",
                    "Google Workspace and other connectors are configured in Settings > Integrations > Prebuilt Connectors.",
                    "User-added pack-based integrations are managed in Settings > Integrations > Custom Integrations.",
                    "MCP servers are managed in Settings > Integrations > MCP Servers.",
                    "Companion devices are managed in Settings > Integrations > Companion Devices.",
                    "Reusable AgentArk procedures or capabilities live in Skills; successful chat-driven skill changes should point back to Skills for management.",
                    "Reusable notes or KB entries live in Memory > Current Memory > Knowledge.",
                    "Uploaded files live in Library > Documents.",
                    "Image or video generation providers live in Settings > Media.",
                    "Moltbook uses the top-level Moltbook page.",
                    "Scheduled work uses Tasks; condition-based monitoring uses Watchers.",
                    "Reflection Daily Digest / Reflect delivery is enabled or disabled from Settings > General > Daily Brief.",
                    "Personal recaps, time-window clustering, and broad month/week/day reflection use Reflect.",
                    "Learning history, impact, live tests, review-only suggestions, and rollback live in Evolve; Sentinel and Evolve switches live in Settings > Advanced.",
                    "Background learning and Sentinel proposals are inspected in Sentinel.",
                    "Behavior debugging uses Trace or Analytics.",
                    "Specialist agents and delegation use Agents.",
                    "Health findings and remediation use Pulse in Ark Core.",
                ],
            },
            AgentArkManualSection {
                label: "semantic intent map",
                items: &[
                    "Do not require product-name phrasing from users. Interpret the underlying intent and route to the matching branded surface by meaning, not by keyword matching.",
                    "Memory owns persistent personal/work knowledge: durable facts, preferences, user data, source attribution, and reusable knowledge-base items.",
                    "Skills owns reusable AgentArk procedures/capabilities, imported SKILL.md packages, enable/disable/test state, required skill secrets, and skill marketplaces.",
                    "Sentinel owns operator decision state: approvals, rejected or snoozed suggestions, background observations, and items waiting for user attention.",
                    "Evolve owns the improvement lifecycle: learning state, experiments, canary or live tests, stable behavior changes, deployment uncertainty, rollback state, and self-evolve controls.",
                    "Reflect owns retrospective understanding: time-window recaps, work patterns, source coverage, activity rhythm, and background-agent activity summaries.",
                    "Pulse owns operational health: diagnostics, findings, runtime state, remediation guidance, and safe fix execution.",
                ],
            },
            AgentArkManualSection {
                label: "answer rule",
                items: &[
                    "For where-do-I-configure-X questions, answer with the exact path first and then the steps.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Companion devices",
        slug: "companion-devices",
        tags: &[
            "companion_devices",
            "devices",
            "iphone",
            "android",
            "desktop",
            "raspberry_pi",
            "iot",
            "pairing",
            "websocket",
            "approvals",
            "security",
        ],
        summary: "Settings > Integrations > Companion Devices pairs scoped devices such as iPhone, Android, desktops, home servers, Raspberry Pi, and custom agents without turning them into admin sessions.",
        sections: &[
            AgentArkManualSection {
                label: "path",
                items: &["Settings > Integrations > Companion Devices."],
            },
            AgentArkManualSection {
                label: "what it is",
                items: &[
                    "Companion devices are paired execution surfaces with explicit capability grants.",
                    "Supported presets include iPhone / iPad, Android phone, macOS / Windows / Linux desktop, home server / mini PC, Raspberry Pi / IoT, and Custom Device.",
                    "Custom devices implement the `agentark-companion-v1` WebSocket protocol and use structured capability ids such as `custom.greenhouse_sensor`.",
                    "Pairing establishes identity; grants define allowed capabilities; approvals allow a specific sensitive action to run now.",
                ],
            },
            AgentArkManualSection {
                label: "security boundaries",
                items: &[
                    "A companion device is not a UI session, admin session, or API key.",
                    "Device tokens are scoped to one device and stored by fingerprint on the AgentArk side.",
                    "Production companion sockets use TLS; plaintext `ws://` is only for local development.",
                    "Paired devices send tokens in WebSocket headers, not JSON message bodies.",
                    "Pairing approval is bound to the claimed device identity before token issue.",
                    "Bundled iOS and Android devices need verified platform attestation before high-risk grants can be approved.",
                    "Custom and desktop devices without platform attestation need an explicit trusted-unattested override before high-risk grants can be approved.",
                    "Requested command scopes must be a subset of both the paired device grant and the caller's current grant.",
                    "Capability reports from a device may show availability but must not expand the approved grant automatically.",
                    "High-risk capabilities such as system commands, files, screenshots, camera, microphone, photos, SMS, location, browser control, and Shortcuts-style actions require fresh approval.",
                    "Command results from a companion are recorded as device-reported unless a future native verifier proves the OS action happened.",
                ],
            },
            AgentArkManualSection {
                label: "setup flow",
                items: &[
                    "Choose a preset or Custom Device.",
                    "Select capabilities before creating the pairing code.",
                    "The companion connects to `/companion/ws`, claims the short-lived pairing session with a stable `device_public_key`, and retries the claim while approval is pending.",
                    "Approve the claimed device identity in the UI.",
                    "The device receives a one-time scoped token through the WebSocket claim result and should store it in the platform keychain or equivalent secret store.",
                    "On later connections, the companion sends `Authorization: Bearer <token>` and `X-AgentArk-Companion-Device: <device_id>` WebSocket headers.",
                    "Active devices send a pulse message for liveness; missed pulses are operational/security signals, not permission changes.",
                ],
            },
            AgentArkManualSection {
                label: "commands",
                items: &[
                    "Companion commands are typed JSON actions with capability, action id, arguments, requested scopes, and resource scope.",
                    "Do not dispatch raw natural-language strings to a companion device.",
                    "If a user asks in natural language, infer structured intent first, validate it against schemas and grants, and ask for clarification when device or action selection is ambiguous.",
                    "High-risk typed commands should create an approval request instead of dispatching directly.",
                ],
            },
            AgentArkManualSection {
                label: "when answering users",
                items: &[
                    "If the user asks where to connect an iPhone or another device, lead with Settings > Integrations > Companion Devices.",
                    "Explain that first-party source for iOS, Android, desktop/headless, and custom devices lives under `clients/companion`; packaging those native apps still requires the platform toolchain.",
                    "If the user asks to add any custom device, explain the Custom Device preset and WebSocket protocol rather than suggesting a generic integration or plugin.",
                    "If a device action is sensitive, mention fresh approval and scoped grants before describing execution.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Mission Control, chat, and approvals",
        slug: "mission-control-chat-and-approvals",
        tags: &[
            "mission_control",
            "chat",
            "inbox",
            "approvals",
            "navigation",
        ],
        summary: "Chat is the main command surface. Mission Control is the daily overview for approvals, live work, highlights, suggestions, and attention items.",
        sections: &[
            AgentArkManualSection {
                label: "entry points",
                items: &[
                    "Chat is where users ask questions, draft, summarize, research, browse, code, call tools, or start multi-step work.",
                    "Mission Control is the daily overview for briefs, suggested next actions, approvals, highlights, and things that need attention.",
                    "Older Inbox references now map to Mission Control attention surfaces.",
                ],
            },
            AgentArkManualSection {
                label: "how to use",
                items: &[
                    "Start in Chat when the user wants help right away.",
                    "Use Mission Control when the user wants a quick view of what is waiting, urgent, or suggested next.",
                    "Return to Mission Control when AgentArk is waiting for approval or has surfaced something that needs review.",
                ],
            },
            AgentArkManualSection {
                label: "what belongs where",
                items: &[
                    "Chat: questions, drafts, summaries, research, browser work, coding, tool execution, and starting new tasks.",
                    "Mission Control: daily overview, suggestions, approvals, alerts, review items, and operational shortcuts.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "Tasks that need approval appear in Mission Control and in the related Tasks flow.",
                    "Completed runs appear in Trace and stop showing as pending in Mission Control.",
                    "Where do I chat with AgentArk maps to Chat.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Chat shortcuts and safe actions",
        slug: "chat-shortcuts-and-safe-actions",
        tags: &[
            "chat_shortcuts",
            "chat",
            "secrets",
            "notifications",
            "delegation",
            "rollback",
            "commands",
        ],
        summary: "These are optional high-frequency shortcuts, not the only valid phrasing. Normal natural-language requests should still route through the usual path.",
        sections: &[
            AgentArkManualSection {
                label: "credentials",
                items: &[
                    "Use the secure credential form shown in chat or the credential fields in Settings.",
                    "Credential flows keep values encrypted and out of normal model-visible arguments and traces.",
                ],
            },
            AgentArkManualSection {
                label: "notifications",
                items: &[
                    "`/notifications pause`.",
                    "`/notifications resume`.",
                    "`/notifications status`.",
                ],
            },
            AgentArkManualSection {
                label: "delegation",
                items: &[
                    "`/delegate <task description>`.",
                    "Use the explicit `/delegate` command when you want to force multi-agent delegation.",
                ],
            },
            AgentArkManualSection {
                label: "rollback",
                items: &[
                    "`/rollback task:<uuid>`.",
                    "`/rollback watcher:<uuid>`.",
                    "`/rollback notification:<id> unread`.",
                    "Natural-language variants like `undo watcher:<uuid>` may also work.",
                ],
            },
            AgentArkManualSection {
                label: "constraints",
                items: &[
                    "These shortcuts are intentionally conservative.",
                    "Do not describe them as the only valid phrasing.",
                    "If the user asks normally in chat instead of using a shortcut, __PRODUCT_NAME__ should still try to help through the usual routing path.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Models and provider setup",
        slug: "models-and-provider-setup",
        tags: &["models", "providers", "llm", "setup", "routing", "research"],
        summary: "Settings > Models configures the model pool for normal chat, coding, research, fallback behavior, and the separate embeddings path.",
        sections: &[
            AgentArkManualSection {
                label: "path",
                items: &["Settings > Models."],
            },
            AgentArkManualSection {
                label: "recommended setup",
                items: &[
                    "Add one Primary model slot first.",
                    "Optionally add Fast, Code, Research, and Fallback slots for role-specific routing.",
                    "Enter provider, model name, base URL if needed, and the API key or credential for each slot.",
                    "Leave Smart routing on if you want __PRODUCT_NAME__ to pick between configured slots automatically.",
                    "Save settings and confirm the slot is enabled.",
                ],
            },
            AgentArkManualSection {
                label: "embeddings",
                items: &[
                    "Settings > Models > Embeddings is separate from chat model slots.",
                    "Default mode is Local using built-in Hugging Face embeddings with `BAAI/bge-small-en-v1.5`.",
                    "Local mode runs in the embeddings sidecar so the control service does not keep the ONNX runtime resident.",
                    "External embeddings are optional and use an OpenAI-compatible embeddings endpoint.",
                    "User-managed Ollama can be used there if the user points __PRODUCT_NAME__ at it explicitly, but Ollama is not bundled for embeddings by default.",
                ],
            },
            AgentArkManualSection {
                label: "roles",
                items: &[
                    "Primary: general default.",
                    "Fast: cheaper and faster simple queries.",
                    "Code: coding-heavy tasks.",
                    "Research: deeper source-backed research flows.",
                    "Fallback: used if the preferred slot fails.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "Settings > Models shows at least one enabled slot.",
                    "The primary slot is runtime-ready, not just saved.",
                    "A normal chat request succeeds after save.",
                    "If a dedicated research slot exists, source-backed research can use it when the user turns on research mode.",
                    "The Embeddings tab shows disabled, a ready local model, or a reachable external endpoint.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "Saved but unavailable at runtime usually means the provider key or base URL is missing or invalid.",
                    "Saved models are not enough if there is no usable primary slot.",
                    "Embeddings health must be checked separately from chat slots.",
                    "If external embeddings are unreachable, fix the base URL, API key, or user-managed service before retrying retrieval-heavy features.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Embeddings and retrieval",
        slug: "embeddings-and-retrieval",
        tags: &[
            "embeddings",
            "retrieval",
            "semantic_search",
            "memory",
            "documents",
            "models",
            "settings",
        ],
        summary: "Settings > Models > Embeddings configures retrieval-backed features such as memory lookup, document search, and other semantic matching flows.",
        sections: &[
            AgentArkManualSection {
                label: "path",
                items: &["Settings > Models > Embeddings."],
            },
            AgentArkManualSection {
                label: "how it works",
                items: &[
                    "Chat model slots and embeddings are separate.",
                    "Chat models power responses, coding, and research.",
                    "Embeddings power retrieval and similarity lookup.",
                    "The default embedding mode is the Local isolated sidecar.",
                ],
            },
            AgentArkManualSection {
                label: "local setup",
                items: &[
                    "Provider: local built-in Hugging Face embeddings.",
                    "Model: `BAAI/bge-small-en-v1.5`.",
                    "This does not require a bundled Ollama service.",
                    "The model is managed by __PRODUCT_NAME__ and initializes on first dense retrieval use.",
                ],
            },
            AgentArkManualSection {
                label: "external embeddings",
                items: &[
                    "External is optional.",
                    "It expects an OpenAI-compatible embeddings endpoint.",
                    "User-managed Ollama can be used here if the user points __PRODUCT_NAME__ at it explicitly.",
                ],
            },
            AgentArkManualSection {
                label: "health",
                items: &[
                    "Ready means the local model or external endpoint is healthy.",
                    "Downloading means the local model is still being prepared.",
                    "Unreachable or Failed means retrieval-backed features are not healthy enough yet.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "The Embeddings tab shows a healthy backend.",
                    "Retrieval-backed features work better than plain keyword fallback.",
                    "Document search, memory lookup, and related context features do not report embedding health failures.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "Chat can work while retrieval still feels weak if embeddings are unhealthy.",
                    "If an external endpoint was saved but is unreachable, fix the base URL, API key, or service.",
                    "Users may expect Ollama to be bundled; clarify that the default path is local Hugging Face embeddings.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Media generation providers",
        slug: "media-generation-providers",
        tags: &[
            "media",
            "images",
            "video",
            "providers",
            "settings",
            "api_keys",
        ],
        summary: "Settings > Media configures image and video generation providers, their API keys, defaults, and fallbacks.",
        sections: &[
            AgentArkManualSection {
                label: "path",
                items: &["Settings > Media."],
            },
            AgentArkManualSection {
                label: "what is here",
                items: &[
                    "Provider API keys for supported media backends.",
                    "Default image provider and image model.",
                    "Fallback image provider.",
                    "Default video provider.",
                    "Fallback video provider.",
                ],
            },
            AgentArkManualSection {
                label: "setup",
                items: &[
                    "Save the API key for the provider you want to use.",
                    "Set the default image provider and image model if you want image generation.",
                    "Set the default video provider if you want video generation.",
                    "Optionally set fallbacks so __PRODUCT_NAME__ can retry on another provider.",
                    "Save settings.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "Settings > Media shows configured providers instead of No media providers.",
                    "Image or video tasks stop failing for missing provider credentials.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "A provider key can exist even when no default provider was selected.",
                    "The default provider can be chosen while the model field is blank or invalid.",
                    "Fallback providers do not replace defaults.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Messaging channels and daily brief",
        slug: "messaging-channels-and-daily-brief",
        tags: &[
            "channels",
            "custom_messaging_channels",
            "telegram",
            "slack",
            "discord",
            "matrix",
            "teams",
            "whatsapp",
            "daily_brief",
            "setup",
        ],
        summary: "Settings > Integrations > Messaging Channels configures where __PRODUCT_NAME__ reaches the user and where the Daily Brief is delivered.",
        sections: &[
            AgentArkManualSection {
                label: "path",
                items: &["Settings > Integrations > Messaging Channels."],
            },
            AgentArkManualSection {
                label: "channels",
                items: &[
                    "Bundled channels include Telegram, Slack, Discord, Matrix, Teams, WhatsApp, Google Chat, Signal, iMessage, LINE, WeChat, QQ, and Email.",
                    "Custom Messaging Channels are user-added outbound delivery channels for webhooks, internal notification services, or provider messaging APIs that are not bundled.",
                    "Unconfigured custom messaging channels are not exposed to the agent's notification chooser until their required credentials are saved.",
                ],
            },
            AgentArkManualSection {
                label: "setup",
                items: &[
                    "Enable the channel you want.",
                    "Fill the required token, webhook, room, team, or recipient fields for that channel.",
                    "For Custom Messaging Channels, ask in chat to add the channel from provider docs or an HTTP/webhook example; then complete the secure credential form in chat or in Settings.",
                    "Never paste secrets into normal chat. Custom channel credentials belong in the inline secure credential prompt or the Settings credential form.",
                    "If a secure credential form appears in chat, users can dismiss it and continue the conversation, then fill the same credentials later from Settings.",
                    "Save settings.",
                    "Check the status card until it changes from Not configured to a ready state.",
                ],
            },
            AgentArkManualSection {
                label: "daily brief",
                items: &[
                    "The Daily Brief section lives in the same Messaging Channels area.",
                    "Pick the time and delivery channel, then enable it only after that channel is ready.",
                    "If the chosen channel is not fully configured, __PRODUCT_NAME__ should warn that delivery is not ready.",
                    "Custom Messaging Channels can be selected for delivery after they are configured and pass the registry readiness check.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "The channel card reads Ready to deliver instead of Needs target or Not configured.",
                    "A Custom Messaging Channel appears under Custom Messaging Channels and shows Ready only after all required secret slots or auth profiles are ready.",
                    "A Daily Brief is only enabled after the selected delivery channel is ready.",
                    "A test run arrives in the selected channel once delivery is fully configured.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "Credentials may be saved without a recipient or room target.",
                    "Daily Brief may be enabled on a channel that is connected but has no delivery target.",
                    "WhatsApp or Telegram can look configured before the user has contacted the bot, leaving no usable destination yet.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Prebuilt connectors and integration quickstarts",
        slug: "prebuilt-connectors-and-integration-quickstarts",
        tags: &["integrations", "connectors", "oauth", "setup", "status"],
        summary: "Settings > Integrations > Prebuilt Connectors is the standard path for built-in service integrations such as Google Workspace, GitHub, Notion, Twilio, Moltbook, and others. User-added pack-based integrations live in the separate Custom Integrations panel.",
        sections: &[
            AgentArkManualSection {
                label: "path",
                items: &["Settings > Integrations > Prebuilt Connectors."],
            },
            AgentArkManualSection {
                label: "standard flow",
                items: &[
                    "Pick the connector you want.",
                    "Save the required secret, token, or OAuth client settings.",
                    "If the connector uses browser auth, finish the sign-in flow.",
                    "Re-check the connector status.",
                ],
            },
            AgentArkManualSection {
                label: "status",
                items: &[
                    "Not configured: required secret or config is missing.",
                    "Needs auth: config is saved, but the browser or OAuth step is incomplete.",
                    "Connected: connector is ready.",
                    "Error: connector responded, but the current config failed.",
                ],
            },
            AgentArkManualSection {
                label: "guidance",
                items: &[
                    "Gmail and Google Workspace have a dedicated bundled doc because provider-side setup is more detailed.",
                    "Moltbook has its own top-level page for ongoing runs even though the integration exists as a connector too.",
                    "Use Custom Integrations instead of Prebuilt Connectors when the service is user-added, imported, or scaffolded as an extension pack.",
                    "Some connectors do not expose a strong background feed, so Watchers or Webhooks may be better for proactive behavior.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "The connector moves to Connected.",
                    "__PRODUCT_NAME__ can use the related tool or action without re-asking for setup.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "A secret can be saved while the dispatch toggle is still off.",
                    "An OAuth client can exist while the redirect or auth flow was never completed.",
                    "The wrong account, tenant, or workspace can be authorized.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Custom integrations and extension packs",
        slug: "custom-integrations-and-extension-packs",
        tags: &[
            "custom_integrations",
            "extension_packs",
            "packs",
            "integrations",
            "install",
            "setup",
            "credentials",
            "secrets",
            "openapi",
            "curl",
        ],
        summary: "Settings > Integrations > Custom Integrations is the user-managed surface for pack-based integrations that are installed, imported, or scaffolded separately from built-in connectors.",
        sections: &[
            AgentArkManualSection {
                label: "path",
                items: &["Settings > Integrations > Custom Integrations."],
            },
            AgentArkManualSection {
                label: "what belongs here",
                items: &[
                    "Use this panel for user-added integrations such as Linear, ClickUp, HubSpot, or internal APIs when they are not a built-in connector.",
                    "Custom integrations are extension-pack based and are managed separately from Prebuilt Connectors.",
                    "Once installed and enabled, pack features can become normal agent-usable actions instead of staying a one-off import.",
                ],
            },
            AgentArkManualSection {
                label: "how to add one",
                items: &[
                    "Ask in chat to install the integration you want, or open Settings > Integrations > Custom Integrations and add it there.",
                    "The panel supports link/path install, bundle upload, and scaffold/import flows from docs, OpenAPI, or cURL examples.",
                    "If the service already exists as a bundled or catalog pack, install that pack first; otherwise scaffold a draft pack and review the generated bindings.",
                ],
            },
            AgentArkManualSection {
                label: "connect and authenticate",
                items: &[
                    "After install, open the custom integration card and complete Setup or Connect.",
                    "OAuth-based packs should open a browser sign-in flow; key- or basic-auth packs should use the secure credential form.",
                    "Never paste secrets into normal chat. Use the secure credential form shown in the conversation or the credential fields in Settings.",
                    "Users may dismiss an inline secure credential prompt and continue chatting; the integration remains configurable from Settings.",
                    "Secrets for custom integrations should be stored encrypted and associated with the integration connection rather than treated as plain chat content.",
                ],
            },
            AgentArkManualSection {
                label: "manage and verify",
                items: &[
                    "Use the card actions or overflow menu to enable or disable the integration, test setup, review runtime status, open Setup again, inspect recent runs, or remove it.",
                    "Normal hot-sync behavior should make a newly connected custom integration usable without restarting the app.",
                    "If a specific pack or runtime still needs restart, __PRODUCT_NAME__ should say that clearly during setup.",
                    "A healthy custom integration appears in Installed, shows a ready-like status, and can be used by the agent without repeating setup each time.",
                ],
            },
            AgentArkManualSection {
                label: "security review",
                items: &[
                    "Pack manifests, plugin bindings, and custom integration actions should declare or derive machine capabilities that map into the shared security vocabulary.",
                    "A single layer can look acceptable while its capabilities combine with another layer into a higher-risk path; cross-layer capability correlation can warn, require approval, or block.",
                    "Review any pack that combines sensitive reads, shell or code execution, file writes, persistence, network delivery, or secret access.",
                ],
            },
            AgentArkManualSection {
                label: "status meanings",
                items: &[
                    "Needs setup: the pack is installed but still missing credentials, OAuth completion, or another required connection step.",
                    "Runtime missing: the pack declares a local CLI or runtime dependency that still needs install or verification.",
                    "Disabled: the pack remains installed but its actions are paused until re-enabled.",
                    "Draft or Needs attention: the pack exists but still needs review, correction, or a connection fix before depending on it.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "Different services can resolve to multiple install paths or auth architectures, so confirm the target when the request is ambiguous.",
                    "A pack may install successfully while the credential or OAuth step is still incomplete.",
                    "Unverified draft packs should be reviewed before using them in production workflows.",
                    "The wrong workspace, tenant, or account can be connected even when the auth flow technically succeeds.",
                ],
            },
            AgentArkManualSection {
                label: "answer rule",
                items: &[
                    "If the user asks how to add an unsupported service integration, answer with the Custom Integrations path first, then the install, connect, verify, and management flow.",
                    "If the service is already built in, route to Prebuilt Connectors instead of describing it as a custom integration.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
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
        summary: "Preferred path: connect Google Workspace once, then use Gmail and Calendar access from the same Google sign-in.",
        sections: &[
            AgentArkManualSection {
                label: "inside __PRODUCT_NAME__",
                items: &[
                    "Open Settings > Integrations > Prebuilt Connectors.",
                    "Find Google Workspace in the connector list.",
                    "Enter the Google OAuth Client ID and Google OAuth Client Secret for this instance.",
                    "In Workspace Bundles, include at least `gmail`; add `calendar` too if needed; then save.",
                    "Click Continue with Google or Connect to open the browser sign-in flow.",
                    "Sign in with the Google account you want __PRODUCT_NAME__ to use and grant the requested scopes.",
                    "Return to __PRODUCT_NAME__ and verify Google Workspace shows connected.",
                ],
            },
            AgentArkManualSection {
                label: "in Google Cloud",
                items: &[
                    "Create or select a Google Cloud project.",
                    "Configure the OAuth consent screen.",
                    "If the app is still in testing, add yourself as a test user.",
                    "Create an OAuth client and copy the client ID and client secret.",
                    "Add the exact redirect URI for this deployment: local installs typically use `http://localhost:8990/oauth/callback`, while internet-facing installs must use `https://<your-host>/oauth/callback`.",
                    "Enable the APIs that match your selected bundles, including Gmail API for Gmail access.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "Google Workspace no longer says not configured or needs auth.",
                    "A connection test passes.",
                    "__PRODUCT_NAME__ can list Gmail or use Google Workspace helper actions without asking for setup again.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "Redirect URI mismatch is the most common issue; it must exactly match the origin and `/oauth/callback` path used by this deployment.",
                    "If the app is in testing and your account is not added as a test user, auth will fail.",
                    "Missing Gmail API or wrong bundle selection will leave Gmail unavailable.",
                ],
            },
            AgentArkManualSection {
                label: "preference",
                items: &[
                    "If the user asks specifically for Gmail access, prefer this Google Workspace path unless they explicitly want the separate legacy Gmail-only connector.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Run Moltbook for the first time",
        slug: "moltbook-first-run",
        tags: &["moltbook", "social", "integrations", "setup", "run"],
        summary: "Moltbook uses its own top-level page for API key setup, status, and run-now controls.",
        sections: &[
            AgentArkManualSection {
                label: "path",
                items: &["Top-level Moltbook page."],
            },
            AgentArkManualSection {
                label: "steps",
                items: &[
                    "Open the Moltbook page from the main navigation.",
                    "Enter the Moltbook API key.",
                    "Save the settings.",
                    "Check the connector or status area on the same page.",
                    "If the page says no API key is configured, save the key first.",
                    "If the stored key cannot connect, fix the key or claim status and try again.",
                    "Click Run now when you want an immediate run.",
                ],
            },
            AgentArkManualSection {
                label: "what the page shows",
                items: &[
                    "Whether Moltbook is enabled.",
                    "Last run time.",
                    "Next run time.",
                    "Recent activity and run logs.",
                    "Whether the stored key is missing or failing authentication.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "After a successful run, the page shows recent Moltbook activity instead of No Moltbook runs yet.",
                    "The run summary shows reads, comments, upvotes, or posts depending on what happened.",
                    "If posting is enabled and safe, the activity log shows run steps and any created post links.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "No API key configured.",
                    "Authentication failed because the key is invalid or the agent has not been claimed yet.",
                    "Disabled mode prevents runs even when config exists.",
                ],
            },
            AgentArkManualSection {
                label: "answer rule",
                items: &[
                    "If the user asks how to run Moltbook, answer with the top-level Moltbook path, key setup, save, run-now, and verification steps.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Library, memory, and documents",
        slug: "library-memory-documents",
        tags: &[
            "library",
            "documents",
            "memory",
            "knowledge",
            "facts",
            "preferences",
            "user_data",
        ],
        summary: "Library, memory, and documents are related but distinct knowledge and retrieval surfaces.",
        sections: &[
            AgentArkManualSection {
                label: "paths",
                items: &[
                    "Library > Documents.",
                    "Memory.",
                    "Memory > Current Memory.",
                    "Memory > Current Memory > Facts.",
                    "Memory > Current Memory > Preferences.",
                    "Memory > Current Memory > User Data.",
                    "Memory > Current Memory > Knowledge.",
                ],
            },
            AgentArkManualSection {
                label: "how to think about them",
                items: &[
                    "Library > Documents is for uploaded files and indexed document context.",
                    "Facts are durable facts the system has stored.",
                    "Preferences are long-lived user preferences and rules.",
                    "User Data is for captured notes, links, and user-supplied structured data.",
                    "Knowledge is for reusable knowledge-base items, including AgentArk manual and capability entries after sync.",
                ],
            },
            AgentArkManualSection {
                label: "when to use each",
                items: &[
                    "Use Library > Documents for file upload and search.",
                    "Use `memory_lookup` when you need durable learned facts, operating constraints, lessons, or procedures during an active request.",
                    "Use Memory > Current Memory > Knowledge for reusable KB entries, notes, or curated instructions.",
                    "Use Facts, Preferences, and User Data when the question is about what __PRODUCT_NAME__ remembers.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "Uploaded files appear in Library > Documents.",
                    "Reusable knowledge items appear in Memory > Current Memory > Knowledge.",
                ],
            },
            AgentArkManualSection {
                label: "common confusion",
                items: &[
                    "Documents are file-centric; Knowledge is reusable KB content.",
                    "Memory is the structured store; Knowledge is only one tab inside that area.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Tasks, watchers, goals, and apps",
        slug: "tasks-watchers-goals-and-apps",
        tags: &["tasks", "watchers", "goals", "apps", "automation", "deploy"],
        summary: "Tasks, Watchers, Goals, and Apps are different top-level operational surfaces.",
        sections: &[
            AgentArkManualSection {
                label: "pages",
                items: &["Tasks.", "Watchers.", "Goals.", "Apps."],
            },
            AgentArkManualSection {
                label: "differences",
                items: &[
                    "Tasks are one-off or recurring work with queue state, approvals, and retries.",
                    "Tasks also surface Input needed when an unattended run is missing critical fields and cannot safely guess them.",
                    "Watchers are background poll-until-condition workflows with timeout or trigger behavior.",
                    "Goals are long-running outcomes tracked over time.",
                    "Apps are built artifacts, deployed surfaces, runtime state, and public or local links.",
                ],
            },
            AgentArkManualSection {
                label: "recommended usage",
                items: &[
                    "Use Tasks when the work should run later or on a schedule.",
                    "Use Watchers when the system should keep checking until something happens.",
                    "Use Goals when the user cares about an outcome that spans multiple runs.",
                    "Use Apps when the agent built or deployed a website, dashboard, or service.",
                ],
            },
            AgentArkManualSection {
                label: "states",
                items: &[
                    "Tasks can be pending, awaiting approval, running, paused, input needed, completed, failed, or cancelled.",
                    "Watchers can be active, paused, triggered, timed out, cancelled, or failed.",
                    "Goals are user-facing outcome trackers even though they run on top of the task system internally.",
                    "Apps can be enabled or disabled, running or stopped, and guarded or public.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "A scheduled job appears in Tasks.",
                    "If a scheduled or background run is blocked on missing fields, it appears in Tasks as Input needed and the task detail lists the missing fields.",
                    "A poll-based monitor appears in Watchers.",
                    "A successful deployment appears in Apps with at least one local or public URL.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "A task can exist but still be waiting for approval in Mission Control.",
                    "A task can move to Input needed because the user was not present and required arguments, targets, or secrets were missing.",
                    "A watcher can be configured while the condition never matches before timeout.",
                    "App files can exist even when the app was not started or the runtime is degraded.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Input needed and unattended runs",
        slug: "input-needed-and-unattended-runs",
        tags: &[
            "input_needed",
            "tasks",
            "notifications",
            "trace",
            "resume",
            "secrets",
            "automation",
        ],
        summary: "Input needed means an unattended or background run could not continue safely because required fields, targets, or secrets were missing.",
        sections: &[
            AgentArkManualSection {
                label: "surfaces",
                items: &[
                    "Main surface: Tasks.",
                    "Related surfaces: Notifications and Trace.",
                ],
            },
            AgentArkManualSection {
                label: "core rule",
                items: &[
                    "If the user is present in chat, __PRODUCT_NAME__ can ask follow-up questions.",
                    "If the run is unattended, scheduled, or backgrounded, __PRODUCT_NAME__ should not guess missing critical inputs.",
                ],
            },
            AgentArkManualSection {
                label: "what it means",
                items: &[
                    "The run paused instead of inventing missing values.",
                    "The task should show the missing fields and guidance for how to fix them.",
                ],
            },
            AgentArkManualSection {
                label: "where to fix",
                items: &[
                    "Non-secret inputs are fixed in Tasks using the task detail and input-edit flow.",
                    "Secret-like inputs belong in Settings > Security / secrets, provider settings, or connector setup.",
                    "Connector-specific credentials must be fixed in the relevant integration settings first.",
                ],
            },
            AgentArkManualSection {
                label: "resume",
                items: &[
                    "Open the affected task in Tasks.",
                    "Review the missing fields shown under Input needed.",
                    "Fix non-secret inputs there, or add the missing secret or config in Settings.",
                    "Resume or retry after the missing inputs are available.",
                ],
            },
            AgentArkManualSection {
                label: "why fail closed",
                items: &[
                    "Guessing can change who or what gets acted on.",
                    "This matters most for sends, deploys, browser actions, account changes, and anything secret-gated.",
                    "Failing closed is safer than performing the wrong action unattended.",
                ],
            },
            AgentArkManualSection {
                label: "examples",
                items: &[
                    "Chat run: __PRODUCT_NAME__ can ask which repo or which thread.",
                    "Scheduled run: it should stop with Input needed if that target was never stored.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
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
        summary: "App deployment is primarily chat-driven, and deployed apps are managed in the top-level Apps page.",
        sections: &[
            AgentArkManualSection {
                label: "main places",
                items: &[
                    "Ask in Chat to build or deploy an app.",
                    "Use Apps to inspect existing deployed apps.",
                    "Use Settings > Advanced > App Deploy Defaults to control the default deploy-guard behavior for new app deploys.",
                ],
            },
            AgentArkManualSection {
                label: "deployment flow",
                items: &[
                    "Ask __PRODUCT_NAME__ in chat to build or deploy the app or repo.",
                    "Let it create files or deploy from a repository source.",
                    "Open the Apps page to inspect the deployed result.",
                    "Use restart, stop, delete, or guard controls from the app card when needed.",
                ],
            },
            AgentArkManualSection {
                label: "access guard",
                items: &[
                    "Access guard protects a deployed app with an access password.",
                    "The default policy for new deploys can be changed in Settings > Advanced > App Deploy Defaults.",
                    "Existing apps can have guard enabled or disabled individually from the Apps page, but public apps must keep it enabled with an explicit access password.",
                    "If guard is enabled, visitors must provide the access password before viewing the app.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "A successful deploy produces a local URL and sometimes a public URL.",
                    "The app appears in the Apps list with runtime state.",
                    "If guard is enabled, the app card says guard is enabled and the visitor flow requests the key.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "Deployment can succeed partially while the runtime fails to start.",
                    "Required secrets or config values may be missing.",
                    "Users may expect a public app while access guard or exposure settings changed the reachable URL flow.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Environment, deployment, and investigation",
        slug: "environment-deployment-and-investigation",
        tags: &[
            "environment",
            "deployment",
            "runtime",
            "permissions",
            "approvals",
            "cpu",
            "memory",
            "integrations",
            "mcp",
            "plugins",
            "observability",
            "sandbox",
        ],
        summary: "Use this when the user asks where this instance is running, what it can access, what is connected, or how to inspect live runtime state safely.",
        sections: &[
            AgentArkManualSection {
                label: "core rule",
                items: &[
                    "Lead with confirmed live state when available, not stale docs alone.",
                    "Use docs to explain where to inspect and how to interpret the result.",
                    "If deployment topology, permissions, or connected systems cannot be confirmed live, say they were not confirmed instead of guessing.",
                ],
            },
            AgentArkManualSection {
                label: "what can be inspected live",
                items: &[
                    "Current workspace, config/data locations, and managed app roots when the runtime exposes them.",
                    "Visible CPU count, sandbox defaults, runtime image clues, container-runtime availability, and app-deploy posture.",
                    "Tasks, watchers, goals, apps, Reflect recaps, traces, analytics, Pulse findings, and security logs.",
                    "Connected integrations, messaging channels, MCP servers, plugins, custom APIs, and reusable actions currently loaded.",
                    "Memory, knowledge, document, and approval/permission state that the instance already tracks.",
                ],
            },
            AgentArkManualSection {
                label: "where to inspect",
                items: &[
                    "Start in Chat with the current runtime access summary and action catalog because they are request-scoped live clues.",
                    "Use Settings > Integrations for messaging channels, prebuilt connectors, MCP servers, plugins, webhooks, and custom APIs.",
                    "Use Memory and Library > Documents for memory and indexed files.",
                    "Use Tasks, Watchers, Goals, Apps, Reflect, Trace, Analytics, and Pulse for durable work and operational investigation.",
                    "Use Settings > Security, Settings > Advanced, and Settings > Observability for approvals, runtime policy, export, and deploy defaults.",
                ],
            },
            AgentArkManualSection {
                label: "permissions and approvals",
                items: &[
                    "Having an action or connector available is not the same thing as having approval to execute every side effect.",
                    "Action permissions can still require approval at execution time even when the tool is installed and visible.",
                    "Mission Control, Tasks, and security-related surfaces are where approval-needed work and guard outcomes should appear.",
                    "Secret values stay encrypted and should never be surfaced as plain-text answers.",
                ],
            },
            AgentArkManualSection {
                label: "deployment and memory caveats",
                items: &[
                    "Public app reachability depends on deployment state, tunnel/exposure setup, and access guard.",
                    "Sandbox memory limits and runtime defaults may be known, but exact host RAM or orchestrator limits are deployment-specific.",
                    "If the user asks for exact machine memory ceilings, verify from the live runtime or container/orchestrator layer rather than inferring from static product docs.",
                ],
            },
            AgentArkManualSection {
                label: "answer order",
                items: &[
                    "What is confirmed live right now.",
                    "Where that state is managed or investigated in __PRODUCT_NAME__.",
                    "What remains unconfirmed and the next inspection step to verify it.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Reflect retrospectives",
        slug: "arkreflect-retrospectives",
        tags: &[
            "arkreflect",
            "reflect",
            "retrospective",
            "recap",
            "clusters",
            "semantic_clusters",
            "working_style",
            "weekly_review",
            "monthly_review",
            "daily_review",
            "arkorbit",
            "memory",
            "analytics",
        ],
        summary: "Reflect is the Ark Core panorama for understanding a selected day, week, or month: where work clustered, what sources contributed, how background agents helped, and what patterns stood out.",
        sections: &[
            AgentArkManualSection {
                label: "where it lives",
                items: &[
                    "Open Ark Core > Reflect in the web UI.",
                    "Use the Day, Week, or Month selector plus the date picker to choose the period.",
                    "The page is intentionally a broad personal recap, not a raw analytics table.",
                ],
            },
            AgentArkManualSection {
                label: "what it shows",
                items: &[
                    "A plain-language narrative summary of the selected period.",
                    "A constellation-style visual of clustered work areas, with larger islands representing more related activity.",
                    "Source coverage across main chat, ArkOrbit chat, memory, procedural patterns, apps, goals, watchers, Sentinel, Pulse, Evolve, and LLM usage.",
                    "Working-style and activity rhythm charts so novice users can see how the period felt without reading raw logs.",
                    "A background-agent lane for work that happened outside direct chat, such as watchers, Sentinel, Pulse, and Evolve activity.",
                    "A Today Status card that shows current-day cached activity and the latest Reflection Daily Digest / Reflect state.",
                    "An examples drawer that keeps technical evidence available without forcing it into the novice-first view.",
                ],
            },
            AgentArkManualSection {
                label: "data model",
                items: &[
                    "Reflect does not store raw per-message chat embeddings.",
                    "It builds retention-managed derived semantic work units from source summaries and metadata, embeds those units, then clusters them for the selected period.",
                    "The `semantic_work_units` table is also used for cross-period related-history lookup so recurring themes can be recognized without scanning every old source row.",
                    "Raw source records remain in their owning systems; Reflect caches only the derived work-unit view needed for fast recaps.",
                ],
            },
            AgentArkManualSection {
                label: "API queries",
                items: &[
                    "Read cached data with `GET /reflect?period=weekly&from=2026-05-01T00:00:00Z&to=2026-05-08T00:00:00Z`.",
                    "Queue a refresh with `POST /reflect/refresh?period=weekly&from=2026-05-01T00:00:00Z&to=2026-05-08T00:00:00Z`.",
                    "Use `GET /reflect?refresh=1&period=monthly&from=2026-05-01T00:00:00Z&to=2026-06-01T00:00:00Z` when the caller wants a cached response and a background refresh request in one call.",
                    "Supported periods are `daily`, `weekly`, and `monthly`; `from` and `to` are RFC3339 timestamps and default to the selected period window when omitted.",
                    "The response includes `clusters`, `source_counts`, `baseline_source_counts`, `embedding_status`, `refresh_status`, `cache_status`, `related_history`, and `unclustered_units`.",
                ],
            },
            AgentArkManualSection {
                label: "runtime behavior",
                items: &[
                    "`GET /reflect` should be treated as a cached read. Do not expect it to scan all sources, embed, and cluster inline.",
                    "Refresh work is single-flight guarded, lease-protected, timeout-bounded, and designed to run in the background when AgentArk is quiet.",
                    "Reflection Daily Digest / Reflect delivery is off by default. Users can enable it in Settings; it then prepares a user-readable recap with the configured model and sends it only when the structured day has meaningful activity.",
                    "If the daily digest gate finds nothing meaningful, no in-app or external notification is sent.",
                    "If cache is empty or stale, show the preparing or stale state calmly and let the refresh job fill the cache; do not present this as a severe warning.",
                    "If semantic embeddings are unavailable, Reflect can still show source-aware activity summaries while semantic grouping catches up.",
                ],
            },
            AgentArkManualSection {
                label: "answer rules",
                items: &[
                    "When the user asks for retrospective understanding of a day, week, or month, point to Reflect before Trace or Analytics unless they specifically need a single run or cost breakdown.",
                    "Use Trace for exact execution steps, Analytics for token or model spend, Pulse for health findings, and Reflect for the broad personal recap.",
                    "Explain Reflect as local and cached by default. Refresh is explicit or background; normal reads should not hang the server.",
                    "Do not describe Reflect as exact phrase matching. Its grouping is based on derived source summaries and embeddings over work units.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Trace, analytics, and Pulse",
        slug: "trace-analytics-and-arkpulse",
        tags: &[
            "trace",
            "analytics",
            "arkreflect",
            "arkpulse",
            "observability",
            "operations",
            "telemetry",
            "prompt_telemetry",
        ],
        summary: "Trace, Analytics, Reflect, and Pulse are separate surfaces for execution history, aggregate metrics, personal retrospectives, and operational health.",
        sections: &[
            AgentArkManualSection {
                label: "pages",
                items: &["Trace.", "Analytics.", "Reflect.", "Pulse."],
            },
            AgentArkManualSection {
                label: "what each one is for",
                items: &[
                    "Trace shows step-by-step execution history for what the agent actually did.",
                    "Analytics shows aggregated usage metrics such as model, channel, token, and cost trends.",
                    "Reflect shows a day/week/month personal recap over clustered work, source coverage, activity rhythm, and background-agent activity.",
                    "Pulse shows operational health and fix guidance across the instance.",
                ],
            },
            AgentArkManualSection {
                label: "prompt telemetry in Trace",
                items: &[
                    "Recent primary-agent runs can include a `Prompt Telemetry` trace step.",
                    "That step is where per-run prompt-size evidence lives: final system prompt chars, tracked section chars, tool count, tool schema chars, and estimated total request size.",
                    "Use Trace for a single run when the question is what was sent to the model or which prompt section dominated that request.",
                ],
            },
            AgentArkManualSection {
                label: "how to use them",
                items: &[
                    "Open Trace when the user asks what the agent did or when a run needs debugging.",
                    "Use trace, conversation, run, and task ids as operational references for correlation; they are not credentials or secrets by themselves.",
                    "Open Analytics when the user asks about usage, volume, model mix, or cost trends.",
                    "Open Reflect when the user wants a broad recap of what happened across chat, ArkOrbit, apps, goals, watchers, memory, and background systems.",
                    "Open Pulse when the user needs operational health or guided remediation for operational findings.",
                ],
            },
            AgentArkManualSection {
                label: "Pulse specifics",
                items: &[
                    "Pulse can surface findings about runtime state, apps, tunnels, and related health issues.",
                    "Some findings support a direct fix path from the UI.",
                    "Advisory-only findings still need manual action.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "A recent run creates a trace entry.",
                    "Analytics shows usage data after real traffic exists.",
                    "Pulse shows either findings or a clean recent run state.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "Analytics is not the right place for a single failed run; use Trace.",
                    "Trace is not the right place for long-term spend trends; use Analytics.",
                    "Reflect is not the raw source of every event; it is a cached derived view for human-readable retrospection.",
                    "Do not treat a redacted placeholder in an operational id field as a valid reference id; it means diagnostic redaction touched data that should be kept as an internal reference.",
                    "Pulse is a higher-level operational guide, not the raw event stream.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Evolve and self-learning",
        slug: "self-learning-and-evolution",
        tags: &[
            "self_learning",
            "evolution",
            "learning",
            "memory",
            "canary",
            "background_learning",
            "sentinel",
            "heuristics",
            "erl",
            "settings",
        ],
        summary: "Evolve is the top-level page for local memory-driven learning, impact, live tests, review-only suggestions, and rollback; Settings > Advanced holds Sentinel and Evolve switches.",
        sections: &[
            AgentArkManualSection {
                label: "learning pipeline",
                items: &[
                    "Completed or degraded runs are recorded as provisional experience runs.",
                    "If the user corrects the result within the correction window, that run can be marked corrected instead of clean success.",
                    "Consolidation turns accepted evidence into durable learned memory such as facts, operating constraints, lessons, and procedures.",
                    "Heuristic reflection turns a completed run into a short transferable lesson so future prompts can reuse what mattered instead of replaying the whole trace.",
                    "Pattern induction turns repeated successful procedures into learned procedural patterns.",
                    "Candidate generation creates draft workflow, strategy, merge, or deprecation candidates for review.",
                    "Draft candidates are suggestions only until they are approved.",
                    "This pipeline is offline-first personalization through local memory, retrieval context, prompts, routing, and policy state; it does not imply continuous base-model weight updates by default.",
                ],
            },
            AgentArkManualSection {
                label: "self-evolve",
                items: &[
                    "Self-evolve focuses on improved routing-policy generation and testing, not silent retraining of the base model.",
                    "Candidate policies can be activated in canary mode so only part of traffic uses them first.",
                    "Replay gate checks help decide whether a candidate is safe to promote.",
                    "Promotion mode, last promotion result, and canary state show rollout stage.",
                ],
            },
            AgentArkManualSection {
                label: "parameter-updating learning",
                items: &[
                    "If a deployment adds learning that changes model parameters, describe it as a separate higher-risk capability.",
                    "Expected controls include documented model lineage, signed artifacts, gated updates or approvals, and poisoning or provenance checks.",
                    "For federated learning setups, require secure aggregation and privacy protections before making stronger claims.",
                ],
            },
            AgentArkManualSection {
                label: "Evolve page",
                items: &[
                    "Evolve > Overview explains current state, whether behavior changed, what happens next, and whether rollback is available.",
                    "Evolve > Results summarizes measured impact from recent prompt, classifier, specialist, and routing changes.",
                    "Evolve > Live tests shows canary rollout, baseline version, candidate version, and gate result for each evolvable surface.",
                    "Evolve > Review queue lists draft learning candidates and keeps them as suggestions until approved.",
                    "Evolve > Review queue may include review-only optimization suggestions; saving one for follow-up records the idea and does not change runtime behavior.",
                    "Evolve > Controls keeps developer-mode canary actions; Settings > Advanced holds Evolve and Sentinel switches plus app deploy defaults.",
                ],
            },
            AgentArkManualSection {
                label: "Sentinel",
                items: &[
                    "Background learning is the live operational status view for heuristic reflection, experience consolidation, pattern induction, and candidate generation.",
                    "Each sub-category shows status, last started or completed times, summary text, and recent counts when available.",
                    "Use Sentinel > Background learning to inspect whether queued learning jobs are running and what changed recently.",
                ],
            },
            AgentArkManualSection {
                label: "answer rules",
                items: &[
                    "If the user asks how self-learning works, explain the pipeline first and then the current instance status.",
                    "If the user asks about the learning system's current state, point to Settings > Advanced for switches and Evolve for current counts, tests, review-only suggestions, and rollback state.",
                    "If the user asks about background learning state, lead with the live Sentinel background learning state and per-job status first.",
                    "If the user asks about prompt cost, prompt size, or prompt telemetry, route them to Trace for per-run evidence and Evolve > Review queue for aggregates and review-only suggestions.",
                    "Use live status rather than stale docs when debugging background learning.",
                    "Do not describe the current product as continuously retraining base model weights unless that deployment explicitly has a parameter-updating feature enabled.",
                    "Keep official product explanation separate from draft candidate content.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Evolve GEPA background optimizer",
        slug: "arkevolve-gepa-background-optimizer",
        tags: &[
            "gepa",
            "dspy",
            "optimizer",
            "prompt_optimization",
            "background_learning",
            "self_evolve",
            "experience_runs",
            "kv_store",
            "cost_budget",
            "docker",
        ],
        summary: "GEPA is the bundled Evolve prompt optimizer. It runs in the background after AgentArk is quiet, uses the same active model configured in Settings > Models, and hands candidates back to Evolve replay and canary gates.",
        sections: &[
            AgentArkManualSection {
                label: "what it is",
                items: &[
                    "GEPA is a DSPy-backed optimizer bridge for prompt and specialist-prompt candidates; it is not base-model weight retraining.",
                    "The Docker image bakes in the Python optimizer runtime at `/opt/agentark-gepa/bin/python` and the bridge package at `/app/bridges/gepa_optimizer`.",
                    "GEPA uses AgentArk's selected primary model/provider credentials from Settings > Models. There is no separate GEPA model picker, API key, local env file, or normal user-run button.",
                    "The normal path is automatic. The UI should surface Background improvement status, queue, evidence, guardrail, and latest result rather than asking a novice user to understand or trigger GEPA.",
                ],
            },
            AgentArkManualSection {
                label: "when it runs",
                items: &[
                    "The scheduler starts about 90 seconds after the HTTP runtime starts, then checks about every 30 minutes.",
                    "It queues work only when learning and self-evolve are enabled, GEPA readiness is clean, the daily budget allows a run, no GEPA job is already pending or running, and AgentArk has been quiet for the quiet window.",
                    "The default quiet window is 5 minutes for automatic scheduling, with an 18 hour cooldown after recent GEPA activity.",
                    "The scheduler requires at least 6 fresh non-provisional experience runs since the latest GEPA activity so it does not optimize on thin or stale evidence.",
                    "Default guardrails reserve at most 1 GEPA run per day, about 1 USD per day, about 0.50 USD per run, and 24 metric calls per run unless changed by operator controls.",
                ],
            },
            AgentArkManualSection {
                label: "data flow",
                items: &[
                    "Rust reads recent `experience_runs`, the current prompt bundle profile, the current specialist prompt bundle profile, benchmark profiles, and recent lineage.",
                    "The bridge writes a redacted run export to `/app/.agentark/self_evolve/gepa/runs/<run_id>/export.json`.",
                    "Python runs `python -m bridges.gepa_optimizer run --export ... --out ...` and writes bounded candidate records to `candidates.jsonl`.",
                    "Rust imports those candidates, sanitizes them, evaluates them through the existing prompt and specialist-prompt evolution engines, and leaves rollout, replay, promotion, and rollback decisions inside Evolve.",
                    "Job status files move through `/app/.agentark/self_evolve/gepa/pending`, `running`, `completed`, and `failed`; detailed run artifacts live under `/app/.agentark/self_evolve/gepa/runs`.",
                ],
            },
            AgentArkManualSection {
                label: "tables and keys",
                items: &[
                    "`experience_runs` is the source evidence table. Inspect `id`, `updated_at`, `success_state`, `correction_state`, `request_text`, `outcome_summary`, `failure_reason`, `prompt_version`, `model_slot`, `metadata`, `consolidated`, and `heuristic_reflected`.",
                    "`kv_store` stores GEPA JSON state under `gepa_optimizer_config_v1`, `gepa_optimizer_auto_state_v1`, `gepa_optimizer_budget_ledger_v1`, and `gepa_optimizer_last_result_v1`.",
                    "`kv_store` also stores the global switches `self_evolve_enabled_v1` and `learning_enabled_v1` that must be on for automatic scheduling.",
                    "Prompt rollout state is stored in `kv_store` under `prompt_bundle_profile_v1`, `prompt_bundle_profile_canary_v1`, `prompt_bundle_canary_state_v1`, and `prompt_bundle_last_result_v1`.",
                    "Specialist prompt rollout state is stored in `kv_store` under `specialist_prompt_bundle_profile_v1`, `specialist_prompt_bundle_profile_canary_v1`, `specialist_prompt_bundle_canary_state_v1`, and `specialist_prompt_bundle_last_result_v1`.",
                ],
            },
            AgentArkManualSection {
                label: "operator queries",
                items: &[
                    "Recent evidence: `SELECT id, updated_at, success_state, correction_state, prompt_version, model_slot FROM experience_runs ORDER BY updated_at DESC LIMIT 20;`",
                    "GEPA state: `SELECT key, updated_at, convert_from(value, 'UTF8')::jsonb AS value FROM kv_store WHERE key IN ('gepa_optimizer_config_v1', 'gepa_optimizer_auto_state_v1', 'gepa_optimizer_budget_ledger_v1', 'gepa_optimizer_last_result_v1');`",
                    "Prompt rollout state: `SELECT key, updated_at, convert_from(value, 'UTF8')::jsonb AS value FROM kv_store WHERE key IN ('prompt_bundle_profile_v1', 'prompt_bundle_profile_canary_v1', 'prompt_bundle_canary_state_v1', 'prompt_bundle_last_result_v1', 'specialist_prompt_bundle_profile_v1', 'specialist_prompt_bundle_profile_canary_v1', 'specialist_prompt_bundle_canary_state_v1', 'specialist_prompt_bundle_last_result_v1');`",
                    "Use the Evolve Background improvement card and Sentinel > Background learning > Prompt tuning before asking a user to inspect SQL directly.",
                ],
            },
            AgentArkManualSection {
                label: "bloat and retention",
                items: &[
                    "GEPA run artifacts are pruned after 30 days and capped to about 80 run directories.",
                    "Completed and failed GEPA status files are pruned with the same retention policy, and stale running jobs are recovered before pruning.",
                    "The GEPA budget ledger keeps about 7 days of entries and caps itself at 512 records.",
                    "The larger long-term database growth source is `experience_runs` and related trace/history tables; use Settings > Data Cleanup for runtime retention instead of adding GEPA-specific table resets.",
                ],
            },
            AgentArkManualSection {
                label: "answer rules",
                items: &[
                    "When asked whether GEPA is ready, prefer live API/UI/DB state over this static documentation.",
                    "Do not tell normal users to create `.env` files, maintain a separate GEPA model configuration, install DSPy manually, or press a GEPA run button.",
                    "If GEPA is not running, explain the gate that blocked it: model readiness, learning switch, daily budget, active work, cooldown, quiet time, or not enough fresh experience.",
                    "Keep GEPA phrasing operator-facing. For novice users call it background improvement or prompt tuning unless they explicitly ask for implementation details.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Prompt telemetry and prompt cost review",
        slug: "prompt-telemetry-and-prompt-cost-review",
        tags: &[
            "telemetry",
            "prompt_telemetry",
            "prompt_cost",
            "tool_schema",
            "observability",
            "trace",
            "evolution",
            "canary",
            "review",
        ],
        summary: "Prompt telemetry measures final prompt and tool-schema size without changing runtime prompt assembly. Use Trace for one run, Evolve > Review queue for aggregates and review items, and observability export for numeric metrics when enabled.",
        sections: &[
            AgentArkManualSection {
                label: "where to inspect",
                items: &[
                    "Trace > Trace Detail for a single run.",
                    "Evolve > Review queue for prompt cost aggregates, largest sections, and review-only optimization proposals.",
                    "Settings > Observability and the external observability backend for exported numeric prompt metrics when export is enabled.",
                ],
            },
            AgentArkManualSection {
                label: "what it measures",
                items: &[
                    "Final system prompt chars after assembly.",
                    "Prompt-section char counts for tracked sections.",
                    "Tool count and serialized tool-schema chars.",
                    "Estimated total request chars and token estimate.",
                ],
            },
            AgentArkManualSection {
                label: "what it does not do",
                items: &[
                    "It does not automatically trim prompt sections or rewrite runtime assembly logic.",
                    "Prompt optimization proposals in Evolve > Review queue are suggestions only until explicitly approved, and approval currently records review state rather than changing runtime prompt behavior.",
                    "Observability export is metrics-only; it should not export raw prompt text, raw tool schemas, or user content.",
                ],
            },
            AgentArkManualSection {
                label: "canary safety",
                items: &[
                    "Prompt, classifier-prompt, and specialist-prompt canaries are watched against resolved experience runs.",
                    "Clear measured regression can disable the canary automatically and raise a notification with the reason.",
                    "Weaker negative signals remain review items in Evolve > Review queue so the operator can choose `Disable canary` or `Keep active`.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "A recent run shows a `Prompt Telemetry` step in Trace.",
                    "Evolve > Review queue shows prompt cost signals after enough runs exist.",
                    "If observability export is enabled, prompt metrics appear as numeric attributes rather than raw prompt content.",
                    "If canary safety triggers, the operator sees a notification and a prompt canary safety item in Evolve > Review queue.",
                ],
            },
            AgentArkManualSection {
                label: "answer rules",
                items: &[
                    "When the user asks where prompt telemetry lives, answer with the exact UI path first and then describe which surface is per-run versus aggregated.",
                    "When the user asks whether telemetry changed runtime behavior, state clearly that measurement is separate from prompt mutation.",
                    "Prefer live Trace, Evolve, and observability state over stale assumptions when debugging cost or prompt-growth questions.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "MCP servers, plugins, webhooks, and custom APIs",
        slug: "mcp-servers-plugins-webhooks-and-custom-apis",
        tags: &[
            "plugins",
            "webhooks",
            "custom_api",
            "integrations",
            "mcp",
            "events",
        ],
        summary: "MCP Servers, Webhooks & APIs, and Plugins are related integration surfaces, but they cover different flows.",
        sections: &[
            AgentArkManualSection {
                label: "paths",
                items: &[
                    "Settings > Integrations > MCP Servers.",
                    "Settings > Integrations > Webhooks & APIs.",
                    "Settings > Integrations > Plugins.",
                ],
            },
            AgentArkManualSection {
                label: "what belongs where",
                items: &[
                    "MCP Servers covers external Model Context Protocol servers that expose tools or resources through configured transports.",
                    "Webhooks & APIs covers incoming webhook sources, webhook events, and imported custom APIs.",
                    "Plugins covers third-party plugin SDK integrations and their subscribed platform events.",
                    "Custom Integrations covers user-added extension-pack integrations that the agent installs or scaffolds as reusable tools.",
                ],
            },
            AgentArkManualSection {
                label: "mcp servers",
                items: &[
                    "Add or edit the MCP server connection in Settings > Integrations > MCP Servers.",
                    "Chat-driven MCP setup should configure the server for this __PRODUCT_NAME__ instance, not for an unrelated desktop client.",
                    "If the MCP server needs bearer, basic, header, query, or other token-style auth, the agent should request credentials through a secure credential form or point to Settings rather than accepting secrets in normal chat.",
                    "Users can dismiss the inline credential form and fill the credential later in Settings > Integrations > MCP Servers.",
                    "Deleting an MCP server should remove the saved config and credentials, refresh the MCP registry, and tolerate already-absent stale registry rows.",
                    "Confirm the transport, auth, tool/resource exposure, and enabled state before relying on it in an agent workflow.",
                    "Treat MCP as an external capability extension, not as an Memory knowledge-base item.",
                ],
            },
            AgentArkManualSection {
                label: "webhooks",
                items: &[
                    "Create or edit a webhook source.",
                    "Save the webhook configuration.",
                    "Use the built-in test action to verify the source.",
                    "Review incoming events and downstream execution in Trace or Tasks.",
                ],
            },
            AgentArkManualSection {
                label: "custom APIs",
                items: &[
                    "Import or configure the custom API in the same Webhooks & APIs area.",
                    "Confirm it is enabled.",
                    "Use it from chat or from flows that depend on that API.",
                ],
            },
            AgentArkManualSection {
                label: "plugins",
                items: &[
                    "Install or edit the plugin.",
                    "Enable only the platform events the plugin should receive.",
                    "Save so plugin actions and test controls become available.",
                ],
            },
            AgentArkManualSection {
                label: "behavior",
                items: &[
                    "Plugins only receive the platform events you explicitly enable.",
                    "Webhooks are ingress; they create or trigger downstream work and are not the execution history themselves.",
                    "Imported custom APIs are distinct from prebuilt connectors even though they share the integration area.",
                    "Pack-based Custom Integrations are also distinct from prebuilt connectors and from raw custom API imports.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "A webhook source passes its test action.",
                    "A custom API appears as enabled after import.",
                    "A plugin appears in the installed plugin list and exposes the expected actions or event subscriptions.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Security, advanced settings, and secrets",
        slug: "security-advanced-and-secrets",
        tags: &[
            "security",
            "advanced",
            "secrets",
            "master_password",
            "sender_verification",
            "alerts",
            "notifications",
        ],
        summary: "Settings > Security covers master password, encrypted secrets, and security logs. Settings > Advanced covers lower-level expert controls.",
        sections: &[
            AgentArkManualSection {
                label: "paths",
                items: &["Settings > Security.", "Settings > Advanced."],
            },
            AgentArkManualSection {
                label: "use Security for",
                items: &[
                    "Master password and secret protection.",
                    "Security status.",
                    "Security logs.",
                ],
            },
            AgentArkManualSection {
                label: "use Advanced for",
                items: &[
                    "Lower-level runtime and integration controls.",
                    "Sender verification and platform-hardening controls.",
                    "Expert-only settings that are not part of normal onboarding.",
                ],
            },
            AgentArkManualSection {
                label: "secret-handling rules",
                items: &[
                    "Prefer settings forms, connector setup, or explicit secret-save flows.",
                    "Do not ask users to paste secrets into normal chat. Installer, integration, messaging-channel, MCP, custom API, and app credential flows should use secure forms or Settings.",
                    "When a secure credential prompt is pending, normal chat must not accept credential values; users can either save through the secure form or dismiss it and continue.",
                    "Treat encrypted secret storage as the source of truth for provider keys, tokens, and connector credentials.",
                ],
            },
            AgentArkManualSection {
                label: "what to explain",
                items: &[
                    "Secrets are stored encrypted and handled separately from normal model generation.",
                    "Internal operational ids such as trace, conversation, run, task, and event ids are correlation references, not credential material.",
                    "Redaction belongs on secret-bearing content and user-visible diagnostics, not on internal reference ids used for lookups or audit correlation.",
                    "Security logs are for audit and review, not just failures.",
                    "A security alert can come from local Web UI chat as well as an external channel; the alert source label tells which surface triggered the guard.",
                    "A local Web UI security alert does not by itself mean Slack, Teams, webhooks, or another external integration is connected.",
                    "Advanced settings should only be changed when the operator knows why the default is insufficient.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "After saving a secret-backed config, the related feature stops showing Not configured.",
                    "Security logs record meaningful security events.",
                    "If a master password change or protected secret flow succeeded, the instance can still read its encrypted settings.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "A secret can exist while another required non-secret field is still missing.",
                    "Users may confuse Security logs with Trace; Trace shows execution while Security shows security-relevant events.",
                    "Advanced settings can be changed without understanding their effect on public exposure or integration trust boundaries.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Swarm, agents, and delegation",
        slug: "swarm-agents-and-delegation",
        tags: &[
            "swarm",
            "agents",
            "delegation",
            "specialists",
            "multi_agent",
        ],
        summary: "The top-level Agents page shows specialist agents and swarm state, but normal users can still trigger delegation directly from chat.",
        sections: &[
            AgentArkManualSection {
                label: "primary surface",
                items: &["Top-level Agents page backed by the swarm system."],
            },
            AgentArkManualSection {
                label: "how it works",
                items: &[
                    "__PRODUCT_NAME__ can delegate parts of complex work to specialist agents.",
                    "The live roster appears in the Agents page.",
                    "Busy or idle state helps show whether specialists are actively working.",
                    "Swarm config controls which specialists exist and how they are provisioned.",
                ],
            },
            AgentArkManualSection {
                label: "what to tell users",
                items: &[
                    "Users can ask in chat for monitoring, escalation, deep research, or multi-step execution and __PRODUCT_NAME__ decides when swarm delegation is appropriate.",
                    "The Agents page is for visibility and management, not the only way to trigger delegation.",
                    "Updating swarm configuration may require restart before a new saved roster fully activates.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "Agents shows registered specialist agents when swarm is configured.",
                    "Swarm status reports enabled and shows live counts.",
                    "During delegated work, agent status moves away from fully idle.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "The instance may have no configured specialist agents.",
                    "A specialist can be saved in config while the process has not restarted to fully apply the new roster.",
                    "Not every task fans out; many are intentionally handled by the main agent alone.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "Browser automation, search, and research",
        slug: "browser-search-and-research",
        tags: &[
            "browser",
            "search",
            "research",
            "web_search",
            "browser_auto",
            "chat",
        ],
        summary: "Web search, research, and browser automation are primarily chat-native workflows rather than settings-first flows.",
        sections: &[
            AgentArkManualSection {
                label: "what they do",
                items: &[
                    "Web search is quick source lookup.",
                    "Research is deeper, slower, and source-backed investigation.",
                    "Browser automation covers website navigation, form filling, reading pages, screenshots, and login-like workflows with user assist when needed.",
                ],
            },
            AgentArkManualSection {
                label: "how to use them",
                items: &[
                    "Ask in Chat for online research or browser work.",
                    "Turn on the Research toggle in chat when the user wants a deeper, source-backed answer.",
                    "Ask for browser actions in plain language when the task needs real website interaction.",
                    "Use Trace afterward to inspect what happened.",
                ],
            },
            AgentArkManualSection {
                label: "behavior",
                items: &[
                    "Research is not the same as a simple web search.",
                    "Browser automation is session-based and can pause for user help on CAPTCHAs, 2FA, or ambiguous pages.",
                    "If the user asks for provider-side setup that drifts over time, keep __PRODUCT_NAME__-specific steps from local docs and verify external console steps with official web sources.",
                ],
            },
            AgentArkManualSection {
                label: "verify",
                items: &[
                    "A research run cites or reflects source-backed findings.",
                    "A browser run leaves trace evidence of navigation, reading, screenshots, or interaction steps.",
                ],
            },
            AgentArkManualSection {
                label: "pitfalls",
                items: &[
                    "The user may want a current answer without enabling research or web use.",
                    "The browser may reach a human checkpoint and need user input before it can continue.",
                    "Users may expect a settings page for everything; browser and research workflows often begin directly in chat.",
                ],
            },
        ],
    },
    AgentArkManualDoc {
        title: "__PRODUCT_NAME__ capabilities overview",
        slug: "capabilities-overview",
        tags: &["capabilities", "features", "overview", "general"],
        summary: "__PRODUCT_NAME__ is a self-hosted personal AI Agent OS for daily life and work that combines private chat, durable memory, tasks, agents, apps, integrations, companion devices, approvals, smart model routing, evolution, and audit trails.",
        sections: &[
            AgentArkManualSection {
                label: "core capabilities",
                items: &[
                    "Personal AI Agent OS workflow across the web UI, CLI, Telegram, WhatsApp, integrations, and companion devices for summaries, drafts, reminders, follow-up, research, app work, and action requests.",
                    "Mission Control for daily overview, approvals, highlights, suggestions, and attention items.",
                    "Memory and personal continuity through durable facts, preferences, user data, uploaded files, reusable knowledge-base items, and local embeddings by default.",
                    "Security and trust controls including encrypted secret handling, model-privacy controls, security logs, approvals, guarded execution, sender verification, and advanced admin settings.",
                    "Smart model routing through Primary, Fast, Code, Research, and Fallback slots so routine OS work can use cheaper capable models while harder work can use stronger specialized models.",
                    "Tasks, Watchers, and Goals for one-off tasks, recurring jobs, and condition-based monitoring.",
                    "Integrations and channels such as Google Workspace, Gmail, Calendar, GitHub, Notion, Twilio, Moltbook, webhooks, plugins, custom APIs, MCP servers, and others depending on configuration.",
                    "Research, browser automation, and documents through web search, deeper source-backed research, website interaction, document inspection, summarization, and grounded answers from indexed content.",
                    "App building and deployment with managed apps, tunnel exposure, restore state, and app status tracking.",
                    "Reflect retrospectives that turn selected days, weeks, and months into local clustered recaps across chat, ArkOrbit, apps, goals, watchers, memory, usage, and background systems.",
                    "Evolve and self-learning through learned memory, learned procedures, background learning, candidate review, replay gates, canary rollout, and impact tracking. This improves retrieval context, prompts, routing, and policy state; it is not silent base-model weight retraining by default.",
                    "Operational power features including swarm agents, execution supervision, traces, analytics, Pulse, plugins, custom APIs, webhooks, and extension packs.",
                ],
            },
            AgentArkManualSection {
                label: "how it evolves over time",
                items: &[
                    "Completed or corrected runs can become evidence for durable memory, lessons, and procedures.",
                    "Background learning can consolidate experience, induce patterns, and create draft candidates for review.",
                    "Self-evolve tests routing-policy candidates through replay gates and canary rollout before promotion.",
                    "Evolve pages show what changed, what helped, what is under test, and what still needs review.",
                    "Do not imply that __PRODUCT_NAME__ silently retrains base model weights unless that deployment explicitly adds parameter-updating learning with documented controls.",
                ],
            },
            AgentArkManualSection {
                label: "security and cost posture",
                items: &[
                    "Secrets are stored encrypted and handled separately from normal model generation.",
                    "Approval, model-privacy, guarded-execution, sender-verification, and security-log surfaces exist for trust and auditability.",
                    "The model pool lets users choose lower-cost fast models for normal OS traffic and keep stronger models for code, research, fallback, or difficult tasks.",
                    "Settings > Models and Analytics help operators inspect the configured model mix and cost trends.",
                ],
            },
            AgentArkManualSection {
                label: "where to look in the UI",
                items: &[
                    "Chat for the main day-to-day workflow.",
                    "Mission Control for overview, approvals, and attention items.",
                    "Settings > Models for LLM and provider setup.",
                    "Settings > Security for master password, logs, and secure handling controls.",
                    "Settings > Integrations > Messaging Channels for delivery channels and Daily Brief setup.",
                    "Settings > Integrations > Prebuilt Connectors for external services.",
                    "Memory for structured memory, provenance, review, and reusable knowledge items.",
                    "Library > Documents for uploaded files and indexed documents.",
                    "Reflect for day/week/month personal recaps, semantic clusters, and background-agent activity summaries.",
                    "Evolve for learning history, impact, canary tests, review, and self-evolve controls.",
                    "Sentinel > Background learning for live reflection, consolidation, pattern induction, and candidate generation status.",
                    "Tasks / Watchers / Goals / Apps / Trace / Analytics / Pulse for deeper operational workflows.",
                ],
            },
            AgentArkManualSection {
                label: "answer rule",
                items: &[
                    "When the user asks what __PRODUCT_NAME__ can do, answer with a short product-specific Markdown list, not a generic chatbot skill list.",
                    "Include evolution, security/trust, model-cost routing, memory/documents, integrations/actions, automation/apps/research, and personal AI Agent OS workflow when answering a broad capabilities question.",
                    "Mention live configured status separately from stable product capability so missing credentials are not confused with missing product features.",
                ],
            },
        ],
    },
];

pub(crate) fn render_agentark_manual_doc(doc: &AgentArkManualDoc) -> String {
    let mut out = String::from(doc.summary.trim());

    for section in doc.sections {
        if section.items.is_empty() {
            continue;
        }
        out.push_str("\n\n");
        out.push_str(section.label);
        out.push(':');
        for item in section.items {
            out.push_str("\n- ");
            out.push_str(item.trim());
        }
    }

    out
}
