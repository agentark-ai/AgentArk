#[derive(Debug, Clone, Copy)]
pub(crate) struct BundledHelpSection {
    pub label: &'static str,
    pub items: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BundledHelpDoc {
    pub title: &'static str,
    pub slug: &'static str,
    pub tags: &'static [&'static str],
    pub summary: &'static str,
    pub sections: &'static [BundledHelpSection],
}

pub(crate) const BUNDLED_HELP_DOCS: &[BundledHelpDoc] = &[
    BundledHelpDoc {
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
            BundledHelpSection {
                label: "docker",
                items: &[
                    "Clone the repo and enter it.",
                    "Run `docker compose up -d --build`.",
                    "Open `http://localhost:8990`.",
                    "Complete the first-run setup.",
                ],
            },
            BundledHelpSection {
                label: "source",
                items: &[
                    "Set `AGENTARK_DATABASE_URL` to a working Postgres instance.",
                    "Build with `cargo build --release`.",
                    "Start with `./target/release/agentark --headless` or launch the normal UI mode.",
                    "Open `http://localhost:8990` if you started headless.",
                ],
            },
            BundledHelpSection {
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
            BundledHelpSection {
                label: "verify",
                items: &[
                    "The web UI opens without the no-model-configured warning.",
                    "Settings save successfully.",
                    "The agent answers a simple chat request.",
                    "Embeddings show a healthy local setup unless the user intentionally configured an external endpoint.",
                    "Security and secret handling are available.",
                    "Configured integrations or channels show connected or configured instead of not configured.",
                ],
            },
            BundledHelpSection {
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
    BundledHelpDoc {
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
            BundledHelpSection {
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
            BundledHelpSection {
                label: "system-owned",
                items: &[
                    "`/app/skills/**`.",
                    "Built-in prompt bundles.",
                    "Frontend/runtime image files.",
                    "Default extension packs.",
                ],
            },
            BundledHelpSection {
                label: "release rule",
                items: &[
                    "Release updates may replace system-owned files.",
                    "Release updates must not mutate user-owned data except through explicit user actions or future versioned migrations with backups.",
                    "`docker compose down -v` is a reset operation because it removes the Docker volumes that hold user-owned data.",
                ],
            },
        ],
    },
    BundledHelpDoc {
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
            BundledHelpSection {
                label: "primary areas",
                items: &[
                    "Mission Control / Chat: main operator workflow, overview, execution, approvals, and alerts.",
                    "Settings > Models: LLM/provider setup, API keys, model behavior, and the separate Embeddings tab.",
                    "Settings > Media: image and video provider keys plus default media providers.",
                    "Settings > Integrations > Messaging Channels: Telegram, WhatsApp, Slack, Discord, Matrix, Teams, and Daily Brief delivery.",
                    "Settings > Integrations > Prebuilt Connectors: Google Workspace, Gmail, Calendar, GitHub, Notion, Twilio, and other connectors.",
                    "Settings > Integrations > Webhooks & APIs: webhook and API-facing integration setup.",
                    "Settings > Integrations > Plugins: plugin-backed integrations.",
                    "Settings > Knowledge > Memory: structured memory, reusable knowledge-base items, preferences, and user data.",
                    "Library > Documents: uploaded files and indexed document context.",
                    "Moltbook: API key setup, status, run-now controls, and activity logs.",
                    "Tasks: scheduled or one-off tasks, including Input needed runs.",
                    "Sentinel: ambient proposals, observations, and Background learning status.",
                    "Watchers: monitor and poll-until workflows.",
                    "Apps: generated or managed apps, deployment, and app status.",
                    "Goals / Agents / Evolution / ArkPulse: long-running outcomes, specialist agents, self-learning status, and operational guidance.",
                    "Trace / Analytics: what the agent did and how it performed.",
                    "Settings > Security / Advanced: security controls and expert settings.",
                ],
            },
            BundledHelpSection {
                label: "routing guidance",
                items: &[
                    "Credentials go to Settings.",
                    "Main conversation happens in Chat.",
                    "Approvals go to Mission Control.",
                    "Google Workspace and other connectors are configured in Settings > Integrations > Prebuilt Connectors.",
                    "Reusable notes or KB entries live in Settings > Knowledge > Memory > Knowledge.",
                    "Uploaded files live in Library > Documents.",
                    "Image or video generation providers live in Settings > Media.",
                    "Moltbook uses the top-level Moltbook page.",
                    "Scheduled work uses Tasks; condition-based monitoring uses Watchers.",
                    "Self-learning history, impact, canary tests, review, and deploy-guard defaults use Evolution.",
                    "Background learning and Sentinel proposals are inspected in Sentinel.",
                    "Behavior debugging uses Trace or Analytics.",
                    "Specialist agents and delegation use Agents.",
                    "Health findings and remediation use ArkPulse.",
                ],
            },
            BundledHelpSection {
                label: "answer rule",
                items: &["For where-do-I-configure-X questions, answer with the exact path first and then the steps."],
            },
        ],
    },
    BundledHelpDoc {
        title: "Mission Control, chat, and approvals",
        slug: "mission-control-chat-and-approvals",
        tags: &["mission_control", "chat", "inbox", "approvals", "navigation"],
        summary: "Chat is the main assistant surface. Mission Control is the daily overview for approvals, highlights, suggestions, and attention items.",
        sections: &[
            BundledHelpSection {
                label: "entry points",
                items: &[
                    "Chat is where users ask questions, draft, summarize, research, browse, code, call tools, or start multi-step work.",
                    "Mission Control is the daily overview for briefs, suggested next actions, approvals, highlights, and things that need attention.",
                    "Older Inbox references now map to Mission Control attention surfaces.",
                ],
            },
            BundledHelpSection {
                label: "how to use",
                items: &[
                    "Start in Chat when the user wants help right away.",
                    "Use Mission Control when the user wants a quick view of what is waiting, urgent, or suggested next.",
                    "Return to Mission Control when the assistant is waiting for approval or has surfaced something that needs review.",
                ],
            },
            BundledHelpSection {
                label: "what belongs where",
                items: &[
                    "Chat: questions, drafts, summaries, research, browser work, coding, tool execution, and starting new tasks.",
                    "Mission Control: daily overview, suggestions, approvals, alerts, review items, and operational shortcuts.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "Tasks that need approval appear in Mission Control and in the related Tasks flow.",
                    "Completed runs appear in Trace and stop showing as pending in Mission Control.",
                    "Where do I talk to the assistant maps to Chat.",
                ],
            },
        ],
    },
    BundledHelpDoc {
        title: "Chat shortcuts and safe command phrases",
        slug: "chat-shortcuts-and-safe-command-phrases",
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
            BundledHelpSection {
                label: "secret save",
                items: &[
                    "Chat: `/setsecret KEY=VALUE`.",
                    "These flows keep the value encrypted and out of normal LLM-visible arguments and traces.",
                ],
            },
            BundledHelpSection {
                label: "notifications",
                items: &["`/notifications pause`.", "`/notifications resume`.", "`/notifications status`."],
            },
            BundledHelpSection {
                label: "delegation",
                items: &[
                    "`/delegate <task description>`.",
                    "Use the explicit `/delegate` command when you want to force multi-agent delegation.",
                ],
            },
            BundledHelpSection {
                label: "rollback",
                items: &[
                    "`/rollback task:<uuid>`.",
                    "`/rollback watcher:<uuid>`.",
                    "`/rollback notification:<id> unread`.",
                    "Natural-language variants like `undo watcher:<uuid>` may also work.",
                ],
            },
            BundledHelpSection {
                label: "constraints",
                items: &[
                    "These shortcuts are intentionally conservative.",
                    "Do not describe them as the only valid phrasing.",
                    "If the user asks normally in chat instead of using a shortcut, __PRODUCT_NAME__ should still try to help through the usual routing path.",
                ],
            },
        ],
    },
    BundledHelpDoc {
        title: "Models and provider setup",
        slug: "models-and-provider-setup",
        tags: &["models", "providers", "llm", "setup", "routing", "research"],
        summary: "Settings > Models configures the model pool for normal chat, coding, research, fallback behavior, and the separate embeddings path.",
        sections: &[
            BundledHelpSection {
                label: "path",
                items: &["Settings > Models."],
            },
            BundledHelpSection {
                label: "recommended setup",
                items: &[
                    "Add one Primary model slot first.",
                    "Optionally add Fast, Code, Research, and Fallback slots for role-specific routing.",
                    "Enter provider, model name, base URL if needed, and the API key or credential for each slot.",
                    "Leave Smart routing on if you want __PRODUCT_NAME__ to pick between configured slots automatically.",
                    "Save settings and confirm the slot is enabled.",
                ],
            },
            BundledHelpSection {
                label: "embeddings",
                items: &[
                    "Settings > Models > Embeddings is separate from chat model slots.",
                    "Default mode is Local using built-in Hugging Face embeddings with `BAAI/bge-small-en-v1.5`.",
                    "External embeddings are optional and use an OpenAI-compatible embeddings endpoint.",
                    "User-managed Ollama can be used there if the user points __PRODUCT_NAME__ at it explicitly, but Ollama is not bundled for embeddings by default.",
                ],
            },
            BundledHelpSection {
                label: "roles",
                items: &[
                    "Primary: general default.",
                    "Fast: cheaper and faster simple queries.",
                    "Code: coding-heavy tasks.",
                    "Research: deeper source-backed research flows.",
                    "Fallback: used if the preferred slot fails.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "Settings > Models shows at least one enabled slot.",
                    "The primary slot is runtime-ready, not just saved.",
                    "A normal chat request succeeds after save.",
                    "If a dedicated research slot exists, source-backed research can use it when the user turns on research mode.",
                    "The Embeddings tab shows either a ready local model or a reachable external endpoint.",
                ],
            },
            BundledHelpSection {
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
    BundledHelpDoc {
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
            BundledHelpSection {
                label: "path",
                items: &["Settings > Models > Embeddings."],
            },
            BundledHelpSection {
                label: "how it works",
                items: &[
                    "Chat model slots and embeddings are separate.",
                    "Chat models power responses, coding, and research.",
                    "Embeddings power retrieval and similarity lookup.",
                    "The default embedding mode is Local.",
                ],
            },
            BundledHelpSection {
                label: "default local setup",
                items: &[
                    "Provider: local built-in Hugging Face embeddings.",
                    "Model: `BAAI/bge-small-en-v1.5`.",
                    "This does not require a bundled Ollama service.",
                    "The model is managed by __PRODUCT_NAME__ and should become ready after download or cache completes.",
                ],
            },
            BundledHelpSection {
                label: "external embeddings",
                items: &[
                    "External is optional.",
                    "It expects an OpenAI-compatible embeddings endpoint.",
                    "User-managed Ollama can be used here if the user points __PRODUCT_NAME__ at it explicitly.",
                ],
            },
            BundledHelpSection {
                label: "health",
                items: &[
                    "Ready means the local model or external endpoint is healthy.",
                    "Downloading means the local model is still being prepared.",
                    "Unreachable or Failed means retrieval-backed features are not healthy enough yet.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "The Embeddings tab shows a healthy backend.",
                    "Retrieval-backed features work better than plain keyword fallback.",
                    "Document search, memory lookup, and related context features do not report embedding health failures.",
                ],
            },
            BundledHelpSection {
                label: "pitfalls",
                items: &[
                    "Chat can work while retrieval still feels weak if embeddings are unhealthy.",
                    "If an external endpoint was saved but is unreachable, fix the base URL, API key, or service.",
                    "Users may expect Ollama to be bundled; clarify that the default path is local Hugging Face embeddings.",
                ],
            },
        ],
    },
    BundledHelpDoc {
        title: "Media generation providers",
        slug: "media-generation-providers",
        tags: &["media", "images", "video", "providers", "settings", "api_keys"],
        summary: "Settings > Media configures image and video generation providers, their API keys, defaults, and fallbacks.",
        sections: &[
            BundledHelpSection {
                label: "path",
                items: &["Settings > Media."],
            },
            BundledHelpSection {
                label: "what is here",
                items: &[
                    "Provider API keys for supported media backends.",
                    "Default image provider and image model.",
                    "Fallback image provider.",
                    "Default video provider.",
                    "Fallback video provider.",
                ],
            },
            BundledHelpSection {
                label: "setup",
                items: &[
                    "Save the API key for the provider you want to use.",
                    "Set the default image provider and image model if you want image generation.",
                    "Set the default video provider if you want video generation.",
                    "Optionally set fallbacks so __PRODUCT_NAME__ can retry on another provider.",
                    "Save settings.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "Settings > Media shows configured providers instead of No media providers.",
                    "Image or video tasks stop failing for missing provider credentials.",
                ],
            },
            BundledHelpSection {
                label: "pitfalls",
                items: &[
                    "A provider key can exist even when no default provider was selected.",
                    "The default provider can be chosen while the model field is blank or invalid.",
                    "Fallback providers do not replace defaults.",
                ],
            },
        ],
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
        summary: "Settings > Integrations > Messaging Channels configures where __PRODUCT_NAME__ reaches the user and where the Daily Brief is delivered.",
        sections: &[
            BundledHelpSection {
                label: "path",
                items: &["Settings > Integrations > Messaging Channels."],
            },
            BundledHelpSection {
                label: "channels",
                items: &["Telegram.", "Slack.", "Discord.", "Matrix.", "Teams.", "WhatsApp."],
            },
            BundledHelpSection {
                label: "setup",
                items: &[
                    "Enable the channel you want.",
                    "Fill the required token, webhook, room, team, or recipient fields for that channel.",
                    "Save settings.",
                    "Check the status card until it changes from Not configured to a ready state.",
                ],
            },
            BundledHelpSection {
                label: "daily brief",
                items: &[
                    "The Daily Brief section lives in the same Messaging Channels area.",
                    "Pick the time and delivery channel, then enable it only after that channel is ready.",
                    "If the chosen channel is not fully configured, __PRODUCT_NAME__ should warn that delivery is not ready.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "The channel card reads Ready to deliver instead of Needs target or Not configured.",
                    "A Daily Brief is only enabled after the selected delivery channel is ready.",
                    "A test run arrives in the selected channel once delivery is fully configured.",
                ],
            },
            BundledHelpSection {
                label: "pitfalls",
                items: &[
                    "Credentials may be saved without a recipient or room target.",
                    "Daily Brief may be enabled on a channel that is connected but has no delivery target.",
                    "WhatsApp or Telegram can look configured before the user has contacted the bot, leaving no usable destination yet.",
                ],
            },
        ],
    },
    BundledHelpDoc {
        title: "Prebuilt connectors and integration quickstarts",
        slug: "prebuilt-connectors-and-integration-quickstarts",
        tags: &["integrations", "connectors", "oauth", "setup", "status"],
        summary: "Settings > Integrations > Prebuilt Connectors is the standard path for built-in service integrations such as Google Workspace, GitHub, Notion, Twilio, Moltbook, and others.",
        sections: &[
            BundledHelpSection {
                label: "path",
                items: &["Settings > Integrations > Prebuilt Connectors."],
            },
            BundledHelpSection {
                label: "standard flow",
                items: &[
                    "Pick the connector you want.",
                    "Save the required secret, token, or OAuth client settings.",
                    "If the connector uses browser auth, finish the sign-in flow.",
                    "Re-check the connector status.",
                ],
            },
            BundledHelpSection {
                label: "status",
                items: &[
                    "Not configured: required secret or config is missing.",
                    "Needs auth: config is saved, but the browser or OAuth step is incomplete.",
                    "Connected: connector is ready.",
                    "Error: connector responded, but the current config failed.",
                ],
            },
            BundledHelpSection {
                label: "guidance",
                items: &[
                    "Gmail and Google Workspace have a dedicated bundled doc because provider-side setup is more detailed.",
                    "Moltbook has its own top-level page for ongoing runs even though the integration exists as a connector too.",
                    "Some connectors do not expose a strong background feed, so Watchers or Webhooks may be better for proactive behavior.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "The connector moves to Connected.",
                    "__PRODUCT_NAME__ can use the related tool or action without re-asking for setup.",
                ],
            },
            BundledHelpSection {
                label: "pitfalls",
                items: &[
                    "A secret can be saved while the dispatch toggle is still off.",
                    "An OAuth client can exist while the redirect or auth flow was never completed.",
                    "The wrong account, tenant, or workspace can be authorized.",
                ],
            },
        ],
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
        summary: "Preferred path: connect Google Workspace once, then use Gmail and Calendar access from the same Google sign-in.",
        sections: &[
            BundledHelpSection {
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
            BundledHelpSection {
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
            BundledHelpSection {
                label: "verify",
                items: &[
                    "Google Workspace no longer says not configured or needs auth.",
                    "A connection test passes.",
                    "__PRODUCT_NAME__ can list Gmail or use Google Workspace helper actions without asking for setup again.",
                ],
            },
            BundledHelpSection {
                label: "pitfalls",
                items: &[
                    "Redirect URI mismatch is the most common issue; it must exactly match the origin and `/oauth/callback` path used by this deployment.",
                    "If the app is in testing and your account is not added as a test user, auth will fail.",
                    "Missing Gmail API or wrong bundle selection will leave Gmail unavailable.",
                ],
            },
            BundledHelpSection {
                label: "preference",
                items: &["If the user asks specifically for Gmail access, prefer this Google Workspace path unless they explicitly want the separate legacy Gmail-only connector."],
            },
        ],
    },
    BundledHelpDoc {
        title: "Run Moltbook for the first time",
        slug: "moltbook-first-run",
        tags: &["moltbook", "social", "integrations", "setup", "run"],
        summary: "Moltbook uses its own top-level page for API key setup, status, and run-now controls.",
        sections: &[
            BundledHelpSection {
                label: "path",
                items: &["Top-level Moltbook page."],
            },
            BundledHelpSection {
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
            BundledHelpSection {
                label: "what the page shows",
                items: &[
                    "Whether Moltbook is enabled.",
                    "Last run time.",
                    "Next run time.",
                    "Recent activity and run logs.",
                    "Whether the stored key is missing or failing authentication.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "After a successful run, the page shows recent Moltbook activity instead of No Moltbook runs yet.",
                    "The run summary shows reads, comments, upvotes, or posts depending on what happened.",
                    "If posting is enabled and safe, the activity log shows run steps and any created post links.",
                ],
            },
            BundledHelpSection {
                label: "pitfalls",
                items: &[
                    "No API key configured.",
                    "Authentication failed because the key is invalid or the agent has not been claimed yet.",
                    "Disabled mode prevents runs even when config exists.",
                ],
            },
            BundledHelpSection {
                label: "answer rule",
                items: &["If the user asks how to run Moltbook, answer with the top-level Moltbook path, key setup, save, run-now, and verification steps."],
            },
        ],
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
        summary: "Library, memory, documents, and MCP are related but distinct surfaces.",
        sections: &[
            BundledHelpSection {
                label: "paths",
                items: &[
                    "Library > Documents.",
                    "Settings > Knowledge > Memory.",
                    "Settings > Knowledge > Memory > Facts.",
                    "Settings > Knowledge > Memory > Preferences.",
                    "Settings > Knowledge > Memory > User Data.",
                    "Settings > Knowledge > Memory > Knowledge.",
                    "Settings > Knowledge > MCP Servers.",
                ],
            },
            BundledHelpSection {
                label: "how to think about them",
                items: &[
                    "Library > Documents is for uploaded files and indexed document context.",
                    "Facts are durable facts the system has stored.",
                    "Preferences are long-lived user preferences and rules.",
                    "User Data is for captured notes, links, and user-supplied structured data.",
                    "Knowledge is for reusable knowledge-base items, including bundled product-help docs after sync.",
                    "MCP Servers are external tool servers that extend what __PRODUCT_NAME__ can access.",
                ],
            },
            BundledHelpSection {
                label: "when to use each",
                items: &[
                    "Use Library > Documents for file upload and search.",
                    "Use Settings > Knowledge > Memory > Knowledge for reusable KB entries, notes, or curated instructions.",
                    "Use Facts, Preferences, and User Data when the question is about what __PRODUCT_NAME__ remembers.",
                    "Use Settings > Knowledge > MCP Servers when you want to add or manage external MCP-backed tools.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "Uploaded files appear in Library > Documents.",
                    "Reusable knowledge items appear in Settings > Knowledge > Memory > Knowledge.",
                    "Enabled MCP servers appear in the MCP list and expose their tools or resources.",
                ],
            },
            BundledHelpSection {
                label: "common confusion",
                items: &[
                    "Documents are file-centric; Knowledge is reusable KB content.",
                    "Memory is the structured store; Knowledge is only one tab inside that area.",
                    "MCP is external capability extension, not the local knowledge base.",
                ],
            },
        ],
    },
    BundledHelpDoc {
        title: "Tasks, watchers, goals, and apps",
        slug: "tasks-watchers-goals-and-apps",
        tags: &["tasks", "watchers", "goals", "apps", "automation", "deploy"],
        summary: "Tasks, Watchers, Goals, and Apps are different top-level operational surfaces.",
        sections: &[
            BundledHelpSection {
                label: "pages",
                items: &["Tasks.", "Watchers.", "Goals.", "Apps."],
            },
            BundledHelpSection {
                label: "differences",
                items: &[
                    "Tasks are one-off or recurring work with queue state, approvals, and retries.",
                    "Tasks also surface Input needed when an unattended run is missing critical fields and cannot safely guess them.",
                    "Watchers are background poll-until-condition workflows with timeout or trigger behavior.",
                    "Goals are long-running outcomes tracked over time.",
                    "Apps are built artifacts, deployed surfaces, runtime state, and public or local links.",
                ],
            },
            BundledHelpSection {
                label: "recommended usage",
                items: &[
                    "Use Tasks when the work should run later or on a schedule.",
                    "Use Watchers when the system should keep checking until something happens.",
                    "Use Goals when the user cares about an outcome that spans multiple runs.",
                    "Use Apps when the agent built or deployed a website, dashboard, or service.",
                ],
            },
            BundledHelpSection {
                label: "states",
                items: &[
                    "Tasks can be pending, awaiting approval, running, paused, input needed, completed, failed, or cancelled.",
                    "Watchers can be active, paused, triggered, timed out, cancelled, or failed.",
                    "Goals are user-facing outcome trackers even though they run on top of the task system internally.",
                    "Apps can be enabled or disabled, running or stopped, and guarded or public.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "A scheduled job appears in Tasks.",
                    "If a scheduled or background run is blocked on missing fields, it appears in Tasks as Input needed and the task detail lists the missing fields.",
                    "A poll-based monitor appears in Watchers.",
                    "A successful deployment appears in Apps with at least one local or public URL.",
                ],
            },
            BundledHelpSection {
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
    BundledHelpDoc {
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
            BundledHelpSection {
                label: "surfaces",
                items: &["Main surface: Tasks.", "Related surfaces: Notifications and Trace."],
            },
            BundledHelpSection {
                label: "core rule",
                items: &[
                    "If the user is present in chat, __PRODUCT_NAME__ can ask follow-up questions.",
                    "If the run is unattended, scheduled, or backgrounded, __PRODUCT_NAME__ should not guess missing critical inputs.",
                ],
            },
            BundledHelpSection {
                label: "what it means",
                items: &[
                    "The run paused instead of inventing missing values.",
                    "The task should show the missing fields and guidance for how to fix them.",
                ],
            },
            BundledHelpSection {
                label: "where to fix",
                items: &[
                    "Non-secret inputs are fixed in Tasks using the task detail and input-edit flow.",
                    "Secret-like inputs belong in Settings > Security / secrets, provider settings, or connector setup.",
                    "Connector-specific credentials must be fixed in the relevant integration settings first.",
                ],
            },
            BundledHelpSection {
                label: "resume",
                items: &[
                    "Open the affected task in Tasks.",
                    "Review the missing fields shown under Input needed.",
                    "Fix non-secret inputs there, or add the missing secret or config in Settings.",
                    "Resume or retry after the missing inputs are available.",
                ],
            },
            BundledHelpSection {
                label: "why fail closed",
                items: &[
                    "Guessing can change who or what gets acted on.",
                    "This matters most for sends, deploys, browser actions, account changes, and anything secret-gated.",
                    "Failing closed is safer than performing the wrong action unattended.",
                ],
            },
            BundledHelpSection {
                label: "examples",
                items: &[
                    "Chat run: __PRODUCT_NAME__ can ask which repo or which thread.",
                    "Scheduled run: it should stop with Input needed if that target was never stored.",
                ],
            },
        ],
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
        summary: "App deployment is primarily chat-driven, and deployed apps are managed in the top-level Apps page.",
        sections: &[
            BundledHelpSection {
                label: "main places",
                items: &[
                    "Ask in Chat to build or deploy an app.",
                    "Use Apps to inspect existing deployed apps.",
                    "Use Evolution > Controls to control the default deploy-guard behavior for new app deploys.",
                ],
            },
            BundledHelpSection {
                label: "deployment flow",
                items: &[
                    "Ask __PRODUCT_NAME__ in chat to build or deploy the app or repo.",
                    "Let it create files or deploy from a repository source.",
                    "Open the Apps page to inspect the deployed result.",
                    "Use restart, stop, delete, or guard controls from the app card when needed.",
                ],
            },
            BundledHelpSection {
                label: "access guard",
                items: &[
                    "Access guard protects a deployed app with an access key.",
                    "The default policy for new deploys can be changed in Evolution > Controls.",
                    "Existing apps can have guard enabled or disabled individually from the Apps page.",
                    "If guard is enabled, visitors must provide the access key before viewing the app.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "A successful deploy produces a local URL and sometimes a public URL.",
                    "The app appears in the Apps list with runtime state.",
                    "If guard is enabled, the app card says guard is enabled and the visitor flow requests the key.",
                ],
            },
            BundledHelpSection {
                label: "pitfalls",
                items: &[
                    "Deployment can succeed partially while the runtime fails to start.",
                    "Required secrets or config values may be missing.",
                    "Users may expect a public app while access guard or exposure settings changed the reachable URL flow.",
                ],
            },
        ],
    },
    BundledHelpDoc {
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
            BundledHelpSection {
                label: "core rule",
                items: &[
                    "Lead with confirmed live state when available, not stale docs alone.",
                    "Use docs to explain where to inspect and how to interpret the result.",
                    "If deployment topology, permissions, or connected systems cannot be confirmed live, say they were not confirmed instead of guessing.",
                ],
            },
            BundledHelpSection {
                label: "what can be inspected live",
                items: &[
                    "Current workspace, config/data locations, and managed app roots when the runtime exposes them.",
                    "Visible CPU count, sandbox defaults, runtime image clues, container-runtime availability, and app-deploy posture.",
                    "Tasks, watchers, goals, apps, traces, analytics, ArkPulse findings, and security logs.",
                    "Connected integrations, messaging channels, MCP servers, plugins, custom APIs, and reusable actions currently loaded.",
                    "Memory, knowledge, document, and approval/permission state that the instance already tracks.",
                ],
            },
            BundledHelpSection {
                label: "where to inspect",
                items: &[
                    "Start in Chat with the current runtime access summary and action catalog because they are request-scoped live clues.",
                    "Use Settings > Integrations > Messaging Channels and Settings > Integrations > Prebuilt Connectors for connected systems.",
                    "Use Settings > Knowledge > Memory and Library > Documents for memory and indexed files.",
                    "Use Tasks, Watchers, Goals, Apps, Trace, Analytics, and ArkPulse for durable work and operational investigation.",
                    "Use Settings > Security, Settings > Advanced, Settings > Observability, and Evolution > Controls for approvals, runtime policy, export, and deploy-guard behavior.",
                ],
            },
            BundledHelpSection {
                label: "permissions and approvals",
                items: &[
                    "Having an action or connector available is not the same thing as having approval to execute every side effect.",
                    "Action permissions can still require approval at execution time even when the tool is installed and visible.",
                    "Mission Control, Tasks, and security-related surfaces are where approval-needed work and guard outcomes should appear.",
                    "Secret values stay encrypted and should never be surfaced as plain-text answers.",
                ],
            },
            BundledHelpSection {
                label: "deployment and memory caveats",
                items: &[
                    "Public app reachability depends on deployment state, tunnel/exposure setup, and access guard.",
                    "Sandbox memory limits and runtime defaults may be known, but exact host RAM or orchestrator limits are deployment-specific.",
                    "If the user asks for exact machine memory ceilings, verify from the live runtime or container/orchestrator layer rather than inferring from static product docs.",
                ],
            },
            BundledHelpSection {
                label: "answer order",
                items: &[
                    "What is confirmed live right now.",
                    "Where that state is managed or investigated in __PRODUCT_NAME__.",
                    "What remains unconfirmed and the next inspection step to verify it.",
                ],
            },
        ],
    },
    BundledHelpDoc {
        title: "Trace, analytics, and ArkPulse",
        slug: "trace-analytics-and-arkpulse",
        tags: &["trace", "analytics", "arkpulse", "observability", "operations"],
        summary: "Trace, Analytics, and ArkPulse are separate top-level surfaces for execution history, aggregate metrics, and operational health.",
        sections: &[
            BundledHelpSection {
                label: "pages",
                items: &["Trace.", "Analytics.", "ArkPulse."],
            },
            BundledHelpSection {
                label: "what each one is for",
                items: &[
                    "Trace shows step-by-step execution history for what the agent actually did.",
                    "Analytics shows aggregated usage metrics such as model, channel, token, and cost trends.",
                    "ArkPulse shows operational health and fix guidance across the instance.",
                ],
            },
            BundledHelpSection {
                label: "how to use them",
                items: &[
                    "Open Trace when the user asks what the agent did or when a run needs debugging.",
                    "Open Analytics when the user asks about usage, volume, model mix, or cost trends.",
                    "Open ArkPulse when the user asks whether the system is healthy or wants guided remediation for operational findings.",
                ],
            },
            BundledHelpSection {
                label: "ArkPulse specifics",
                items: &[
                    "ArkPulse can surface findings about runtime state, apps, tunnels, and related health issues.",
                    "Some findings support a direct fix path from the UI.",
                    "Advisory-only findings still need manual action.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "A recent run creates a trace entry.",
                    "Analytics shows usage data after real traffic exists.",
                    "ArkPulse shows either findings or a clean recent run state.",
                ],
            },
            BundledHelpSection {
                label: "pitfalls",
                items: &[
                    "Analytics is not the right place for a single failed run; use Trace.",
                    "Trace is not the right place for long-term spend trends; use Analytics.",
                    "ArkPulse is a higher-level operational guide, not the raw event stream.",
                ],
            },
        ],
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
            "background_learning",
            "sentinel",
            "settings",
        ],
        summary: "Evolution is the top-level page for local memory-driven learning, impact, canary tests, review, and self-evolve controls; Sentinel shows live Background learning status.",
        sections: &[
            BundledHelpSection {
                label: "learning pipeline",
                items: &[
                    "Completed or degraded runs are recorded as provisional experience runs.",
                    "If the user corrects the result within the correction window, that run can be marked corrected instead of clean success.",
                    "Consolidation turns accepted evidence into durable learned memory such as facts, operating constraints, lessons, and procedures.",
                    "Pattern induction turns repeated successful procedures into learned procedural patterns.",
                    "Candidate generation creates draft workflow, strategy, merge, or deprecation candidates for review.",
                    "Draft candidates are suggestions only until they are approved.",
                    "This pipeline is offline-first personalization through local memory, retrieval context, prompts, routing, and policy state; it does not imply continuous base-model weight updates by default.",
                ],
            },
            BundledHelpSection {
                label: "self-evolve",
                items: &[
                    "Self-evolve focuses on improved routing-policy generation and testing, not silent retraining of the base model.",
                    "Candidate policies can be activated in canary mode so only part of traffic uses them first.",
                    "Replay gate checks help decide whether a candidate is safe to promote.",
                    "Promotion mode, last promotion result, and canary state show rollout stage.",
                ],
            },
            BundledHelpSection {
                label: "parameter-updating learning",
                items: &[
                    "If a deployment adds learning that changes model parameters, describe it as a separate higher-risk capability.",
                    "Expected controls include documented model lineage, signed artifacts, gated updates or approvals, and poisoning or provenance checks.",
                    "For federated learning setups, require secure aggregation and privacy protections before making stronger claims.",
                ],
            },
            BundledHelpSection {
                label: "Evolution page",
                items: &[
                    "Evolution > What happened explains recent tested or promoted changes in plain language.",
                    "Evolution > What helped summarizes measured impact from recent prompt, classifier, specialist, and routing changes.",
                    "Evolution > Tests running shows canary rollout, baseline version, candidate version, and gate result for each evolvable surface.",
                    "Evolution > Review lists draft learning candidates and keeps them as suggestions until approved.",
                    "Evolution > Controls includes self-evolve, learning, local-only learning, deploy-guard default, and developer-mode canary actions.",
                ],
            },
            BundledHelpSection {
                label: "Sentinel",
                items: &[
                    "Background learning is the live operational status view for reflection pass, memory consolidation, experience consolidation, pattern induction, and candidate generation.",
                    "Each sub-category shows status, last started or completed times, summary text, and recent counts when available.",
                    "Use Sentinel > Background learning to inspect whether queued learning jobs are running and what changed recently.",
                ],
            },
            BundledHelpSection {
                label: "answer rules",
                items: &[
                    "If the user asks how self-learning works, explain the pipeline first and then the current instance status.",
                    "If the user asks whether it is enabled or what it has learned, point to Evolution first, report current toggles and counts, then explain the meaning.",
                    "If the user asks about background learning status or why it is not running, lead with the live Sentinel background learning state and per-job status first.",
                    "Use live status rather than stale docs when debugging background learning.",
                    "Do not describe the current product as continuously retraining base model weights unless that deployment explicitly has a parameter-updating feature enabled.",
                    "Keep official product explanation separate from draft candidate content.",
                ],
            },
        ],
    },
    BundledHelpDoc {
        title: "Plugins, webhooks, and custom APIs",
        slug: "plugins-webhooks-and-custom-apis",
        tags: &["plugins", "webhooks", "custom_api", "integrations", "mcp", "events"],
        summary: "Webhooks & APIs and Plugins are related integration surfaces, but they cover different flows.",
        sections: &[
            BundledHelpSection {
                label: "paths",
                items: &[
                    "Settings > Integrations > Webhooks & APIs.",
                    "Settings > Integrations > Plugins.",
                ],
            },
            BundledHelpSection {
                label: "what belongs where",
                items: &[
                    "Webhooks & APIs covers incoming webhook sources, webhook events, and imported custom APIs.",
                    "Plugins covers third-party plugin SDK integrations and their subscribed platform events.",
                ],
            },
            BundledHelpSection {
                label: "webhooks",
                items: &[
                    "Create or edit a webhook source.",
                    "Save the webhook configuration.",
                    "Use the built-in test action to verify the source.",
                    "Review incoming events and downstream execution in Trace or Tasks.",
                ],
            },
            BundledHelpSection {
                label: "custom APIs",
                items: &[
                    "Import or configure the custom API in the same Webhooks & APIs area.",
                    "Confirm it is enabled.",
                    "Use it from chat or from flows that depend on that API.",
                ],
            },
            BundledHelpSection {
                label: "plugins",
                items: &[
                    "Install or edit the plugin.",
                    "Enable only the platform events the plugin should receive.",
                    "Save so plugin actions and test controls become available.",
                ],
            },
            BundledHelpSection {
                label: "behavior",
                items: &[
                    "Plugins only receive the platform events you explicitly enable.",
                    "Webhooks are ingress; they create or trigger downstream work and are not the execution history themselves.",
                    "Imported custom APIs are distinct from prebuilt connectors even though they share the integration area.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "A webhook source passes its test action.",
                    "A custom API appears as enabled after import.",
                    "A plugin appears in the installed plugin list and exposes the expected actions or event subscriptions.",
                ],
            },
        ],
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
        summary: "Settings > Security covers master password, encrypted secrets, and security logs. Settings > Advanced covers lower-level expert controls.",
        sections: &[
            BundledHelpSection {
                label: "paths",
                items: &["Settings > Security.", "Settings > Advanced."],
            },
            BundledHelpSection {
                label: "use Security for",
                items: &["Master password and secret protection.", "Security status.", "Security logs."],
            },
            BundledHelpSection {
                label: "use Advanced for",
                items: &[
                    "Lower-level runtime and integration controls.",
                    "Sender verification and platform-hardening controls.",
                    "Expert-only settings that are not part of normal onboarding.",
                ],
            },
            BundledHelpSection {
                label: "secret-handling rules",
                items: &[
                    "Prefer settings forms, connector setup, or explicit secret-save flows.",
                    "Do not ask users to paste secrets into general chat unless the flow explicitly supports secure handling.",
                    "Treat encrypted secret storage as the source of truth for provider keys, tokens, and connector credentials.",
                ],
            },
            BundledHelpSection {
                label: "what to explain",
                items: &[
                    "Secrets are stored encrypted and handled separately from normal model generation.",
                    "Security logs are for audit and review, not just failures.",
                    "Advanced settings should only be changed when the operator knows why the default is insufficient.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "After saving a secret-backed config, the related feature stops showing Not configured.",
                    "Security logs record meaningful security events.",
                    "If a master password change or protected secret flow succeeded, the instance can still read its encrypted settings.",
                ],
            },
            BundledHelpSection {
                label: "pitfalls",
                items: &[
                    "A secret can exist while another required non-secret field is still missing.",
                    "Users may confuse Security logs with Trace; Trace shows execution while Security shows security-relevant events.",
                    "Advanced settings can be changed without understanding their effect on public exposure or integration trust boundaries.",
                ],
            },
        ],
    },
    BundledHelpDoc {
        title: "Swarm, agents, and delegation",
        slug: "swarm-agents-and-delegation",
        tags: &["swarm", "agents", "delegation", "specialists", "multi_agent"],
        summary: "The top-level Agents page shows specialist agents and swarm state, but normal users can still trigger delegation directly from chat.",
        sections: &[
            BundledHelpSection {
                label: "primary surface",
                items: &["Top-level Agents page backed by the swarm system."],
            },
            BundledHelpSection {
                label: "how it works",
                items: &[
                    "__PRODUCT_NAME__ can delegate parts of complex work to specialist agents.",
                    "The live roster appears in the Agents page.",
                    "Busy or idle state helps show whether specialists are actively working.",
                    "Swarm config controls which specialists exist and how they are provisioned.",
                ],
            },
            BundledHelpSection {
                label: "what to tell users",
                items: &[
                    "Users can ask in chat for monitoring, escalation, deep research, or multi-step execution and __PRODUCT_NAME__ decides when swarm delegation is appropriate.",
                    "The Agents page is for visibility and management, not the only way to trigger delegation.",
                    "Updating swarm configuration may require restart before a new saved roster fully activates.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "Agents shows registered specialist agents when swarm is configured.",
                    "Swarm status reports enabled and shows live counts.",
                    "During delegated work, agent status moves away from fully idle.",
                ],
            },
            BundledHelpSection {
                label: "pitfalls",
                items: &[
                    "The instance may have no configured specialist agents.",
                    "A specialist can be saved in config while the process has not restarted to fully apply the new roster.",
                    "Not every task fans out; many are intentionally handled by the main agent alone.",
                ],
            },
        ],
    },
    BundledHelpDoc {
        title: "Browser automation, search, and research",
        slug: "browser-search-and-research",
        tags: &["browser", "search", "research", "web_search", "browser_auto", "chat"],
        summary: "Web search, research, and browser automation are primarily chat-native workflows rather than settings-first flows.",
        sections: &[
            BundledHelpSection {
                label: "what they do",
                items: &[
                    "Web search is quick source lookup.",
                    "Research is deeper, slower, and source-backed investigation.",
                    "Browser automation covers website navigation, form filling, reading pages, screenshots, and login-like workflows with user assist when needed.",
                ],
            },
            BundledHelpSection {
                label: "how to use them",
                items: &[
                    "Ask in Chat for online research or browser work.",
                    "Turn on the Research toggle in chat when the user wants a deeper, source-backed answer.",
                    "Ask for browser actions in plain language when the task needs real website interaction.",
                    "Use Trace afterward to inspect what happened.",
                ],
            },
            BundledHelpSection {
                label: "behavior",
                items: &[
                    "Research is not the same as a simple web search.",
                    "Browser automation is session-based and can pause for user help on CAPTCHAs, 2FA, or ambiguous pages.",
                    "If the user asks for provider-side setup that drifts over time, keep __PRODUCT_NAME__-specific steps from local docs and verify external console steps with official web sources.",
                ],
            },
            BundledHelpSection {
                label: "verify",
                items: &[
                    "A research run cites or reflects source-backed findings.",
                    "A browser run leaves trace evidence of navigation, reading, screenshots, or interaction steps.",
                ],
            },
            BundledHelpSection {
                label: "pitfalls",
                items: &[
                    "The user may want a current answer without enabling research or web use.",
                    "The browser may reach a human checkpoint and need user input before it can continue.",
                    "Users may expect a settings page for everything; browser and research workflows often begin directly in chat.",
                ],
            },
        ],
    },
    BundledHelpDoc {
        title: "__PRODUCT_NAME__ capabilities overview",
        slug: "capabilities-overview",
        tags: &["capabilities", "features", "overview", "general"],
        summary: "__PRODUCT_NAME__ is a self-hosted personal AI assistant for daily life and work that combines private chat, durable memory, daily briefs, secure secrets, approvals, smart model routing, evolution, and optional power-user automation.",
        sections: &[
            BundledHelpSection {
                label: "core capabilities",
                items: &[
                    "Daily personal-assistant workflow across the web UI, CLI, Telegram, and WhatsApp for summaries, drafts, reminders, follow-up, research, and action requests.",
                    "Mission Control for daily overview, approvals, highlights, suggestions, and attention items.",
                    "Memory and personal continuity through durable facts, preferences, user data, uploaded files, reusable knowledge-base items, and local embeddings by default.",
                    "Security and trust controls including encrypted secret handling, model-privacy controls, security logs, approvals, guarded execution, sender verification, and advanced admin settings.",
                    "Smart model routing through Primary, Fast, Code, Research, and Fallback slots so routine personal-assistant work can use cheaper capable models while harder work can use stronger specialized models.",
                    "Tasks, Watchers, and Goals for one-off tasks, recurring jobs, and condition-based monitoring.",
                    "Integrations and channels such as Google Workspace, Gmail, Calendar, GitHub, Notion, Twilio, Moltbook, webhooks, plugins, custom APIs, MCP servers, and others depending on configuration.",
                    "Research, browser automation, and documents through web search, deeper source-backed research, website interaction, document inspection, summarization, and grounded answers from indexed content.",
                    "App building and deployment with managed apps, tunnel exposure, restore state, and app status tracking.",
                    "Evolution and self-learning through learned memory, learned procedures, background learning, candidate review, replay gates, canary rollout, and impact tracking. This improves retrieval context, prompts, routing, and policy state; it is not silent base-model weight retraining by default.",
                    "Operational power features including swarm agents, execution supervision, traces, analytics, ArkPulse, plugins, custom APIs, webhooks, and extension packs.",
                ],
            },
            BundledHelpSection {
                label: "how it evolves over time",
                items: &[
                    "Completed or corrected runs can become evidence for durable memory, lessons, and procedures.",
                    "Background learning can consolidate experience, induce patterns, and create draft candidates for review.",
                    "Self-evolve tests routing-policy candidates through replay gates and canary rollout before promotion.",
                    "Evolution pages show what changed, what helped, what is under test, and what still needs review.",
                    "Do not imply that __PRODUCT_NAME__ silently retrains base model weights unless that deployment explicitly adds parameter-updating learning with documented controls.",
                ],
            },
            BundledHelpSection {
                label: "security and cost posture",
                items: &[
                    "Secrets are stored encrypted and handled separately from normal model generation.",
                    "Approval, model-privacy, guarded-execution, sender-verification, and security-log surfaces exist for trust and auditability.",
                    "The model pool lets users choose lower-cost fast models for normal personal-assistant traffic and keep stronger models for code, research, fallback, or difficult tasks.",
                    "Settings > Models and Analytics help operators inspect the configured model mix and cost trends.",
                ],
            },
            BundledHelpSection {
                label: "where to look in the UI",
                items: &[
                    "Chat for the main day-to-day workflow.",
                    "Mission Control for overview, approvals, and attention items.",
                    "Settings > Models for LLM and provider setup.",
                    "Settings > Security for master password, logs, and secure handling controls.",
                    "Settings > Integrations > Messaging Channels for delivery channels and Daily Brief setup.",
                    "Settings > Integrations > Prebuilt Connectors for external services.",
                    "Settings > Knowledge > Memory for structured memory and reusable knowledge items.",
                    "Library > Documents for uploaded files and indexed documents.",
                    "Evolution for learning history, impact, canary tests, review, and self-evolve controls.",
                    "Sentinel > Background learning for live reflection, consolidation, pattern induction, and candidate generation status.",
                    "Tasks / Watchers / Goals / Apps / Trace / Analytics / ArkPulse for deeper operational workflows.",
                ],
            },
            BundledHelpSection {
                label: "answer rule",
                items: &[
                    "When the user asks what __PRODUCT_NAME__ can do, answer with a short product-specific Markdown list, not a generic AI assistant skill list.",
                    "Include evolution, security/trust, model-cost routing, memory/documents, integrations/actions, automation/apps/research, and daily personal-assistant workflow when answering a broad capabilities question.",
                    "Mention live configured status separately from stable product capability so missing credentials are not confused with missing product features.",
                ],
            },
        ],
    },
];

pub(crate) fn render_bundled_help_doc(doc: &BundledHelpDoc) -> String {
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
