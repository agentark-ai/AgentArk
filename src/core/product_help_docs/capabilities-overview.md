# AgentArk capabilities overview

AgentArk is a self-hosted agent runtime with chat, memory, actions, integrations, scheduling, and operations surfaces in one product.

Core capabilities:

1. Chat and execution
Talk to AgentArk in the web UI, CLI, Telegram, and WhatsApp. It can answer directly, call tools, or execute multi-step work.

2. Mission Control and approvals
Mission Control gives a top-level operational overview, suggested next actions, and the attention surfaces for approvals and review items.

3. Memory and knowledge
AgentArk stores durable user facts, preferences, user data, uploaded files, and reusable knowledge-base items. Settings > Knowledge > Memory is where structured memory and reusable knowledge-base items live, and Library > Documents is where uploaded files live.

4. Tasks, watchers, and automation
You can schedule one-off tasks, recurring jobs, and watchers that poll for a condition and act when it becomes true.

5. App building and deployment
AgentArk can write app files, deploy managed apps, expose them through the configured tunnel provider, and track app status.

6. Browser and operator workflows
It can automate websites, inspect pages, fill forms, and continue after user help for 2FA or ambiguous UI states.

7. Integrations and channels
It supports external integrations such as Google Workspace, Gmail, Calendar, GitHub, Notion, Twilio, Moltbook, and others, depending on what is configured in this instance.

8. Research and documents
It can search, summarize, inspect uploaded documents, and answer grounded questions from indexed content.

9. Swarm and self-improvement
It includes specialist agents, execution supervision, traces, analytics, and self-evolve features.

10. Plugins, webhooks, and custom APIs
AgentArk can accept inbound webhooks, import custom APIs, and run third-party plugins that subscribe to platform events.

11. Security and administration
It includes security logs, encrypted secret handling, advanced controls, observability settings, ArkPulse operational checks, and guarded app deployment.

Where to look in the UI:

- Mission Control / Chat for the primary user workflow
- Settings > Models for LLM/provider setup
- Settings > Media for image/video provider setup
- Settings > Integrations > Prebuilt Connectors and Messaging Channels for external services
- Settings > Integrations > Webhooks & APIs and Plugins for extensibility
- Moltbook for Moltbook-specific setup and runs
- Settings > Knowledge > Memory for reusable knowledge-base items
- Library > Documents for uploaded files and indexed docs
- Tasks / Watchers / Goals / Apps / Trace / Analytics / ArkPulse for operational workflows
- Settings > Security / Advanced / Evolution for security and admin controls

When a user asks "what can AgentArk do?" answer with the areas above, then narrow to the exact feature path they need next.
