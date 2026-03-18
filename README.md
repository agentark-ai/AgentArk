<p align="center">
  <img src="assets/logo.svg" alt="AgentArk Logo" width="420" height="240">
</p>

<h1 align="center">AgentArk 🚀</h1>


<p align="center">
  <em><strong>T</strong>hink. <strong>A</strong>ct. <strong>R</strong>emember. <strong>S</strong>ecurely.</em>
</p>

<p align="center">
  <strong>Autonomous agent control plane. Secure by design, not by choice.</strong><br>
  ⚡ Starts in &lt;50ms · ~34MB RAM · Runs on any hardware
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="License"></a>
  <a href="https://github.com/agentark-ai/AgentArk/stargazers"><img src="https://img.shields.io/github/stars/agentark-ai/AgentArk?style=flat" alt="Stars"></a>
  <a href="https://github.com/agentark-ai/AgentArk/issues"><img src="https://img.shields.io/github/issues/agentark-ai/AgentArk" alt="Issues"></a>
  <a href="https://github.com/agentark-ai/AgentArk/pulls"><img src="https://img.shields.io/github/issues-pr/agentark-ai/AgentArk" alt="PRs"></a>
</p>

<p align="center">
  <a href="#what-is-agentark">Overview</a> |
  <a href="#the-idea">Idea</a> |
  <a href="#install">Install</a> |
  <a href="#high-level-benchmark">Compare</a> |
  <a href="#features">Features</a> |
  <a href="#configuration">Configuration</a> |
  <a href="#api">API</a>
</p>

<p align="center">
  <strong>Quick Routes:</strong>
  <a href="#what-is-agentark">What is it?</a> ·
  <a href="#the-idea">Idea</a> ·
  <a href="#install">Install</a> ·
  <a href="#high-level-benchmark">Compare</a> ·
  <a href="#architecture">Architecture</a> ·
  <a href="#troubleshooting">Troubleshoot</a> ·
  <a href="#why-rust">Why Rust?</a> ·
  <a href="#benchmark-snapshot">Benchmarks</a> ·
  <a href="#contributing">Contribute</a>
</p>

<p align="center">
  <strong>Parallel thinking · sub-agent orchestration · self-evolve engine · encrypted storage</strong><br>
  Deploy anywhere. Connect any LLM. Automate everything.
</p>

<p align="center">
  <code>AES-256-GCM encryption · WASM sandbox · action security guard · prompt injection protection · verifiable action history</code>
</p>

---

## What Is AgentArk?

Your AI doesn't need another chat window. It needs a **control plane**.

AgentArk is a self-hosted agent runtime that turns any LLM into a persistent, autonomous operator - one that remembers everything, executes real actions, monitors the world while you sleep, and gets better at its job over time.

It runs on your machine. Your data never leaves. You own the agent.

### Why AgentArk feels different

Most agent projects stop at "chat + tools + cron". AgentArk is built as a full control plane:

| Category | Typical lightweight agent repos | AgentArk |
|:--|:--|:--|
| **Execution model** | Usually one shape: chat loop plus ad hoc tools | One runtime for chat runs, tasks, watchers, apps, integrations, and reusable skills |
| **Autonomy** | Mostly prompt-driven behavior inside one long LLM loop | Structural autonomy with supervision, retries, validation, trace history, and execution proofs |
| **State** | Conversation-local context, maybe a memory file | Durable user facts, goals, apps, task state, watcher state, logs, and integration state |
| **Apps** | Often code generation only, or separate from the agent runtime | Builds, deploys, monitors, exposes, and revisits apps from the same conversation |
| **Capability growth** | Many frameworks can add skills/tools from chat or plugins, but the path is often framework-specific | Built-ins plus first-class capability acquisition in the runtime, including scaffolding actions/integrations from API specs or docs |
| **Operations UX** | Black-box "agent thinking..." with weak runtime visibility | Tasks, Watchers, Apps, Trace, Analytics, ArkPulse, and channel-aware activity surfaces |
| **Security model** | Often bolted on around prompts and API calls | AES-256-GCM at rest, Argon2id key derivation for master-password mode, encrypted `secrets.enc`, keyfile fallback, sandboxing, approvals, security events, and verifiable history |

That makes AgentArk closer to an operating system for agents than a prompt wrapper with a memory file.

Security is not just a checkbox here. Sensitive config is split from readable config: normal settings live in `config.toml`, while API keys, OAuth tokens, and custom secrets are stored encrypted in `secrets.enc`. When a master password is set, AgentArk derives the encryption key with **Argon2id** and uses **AES-256-GCM** for encryption at rest. Without a master password, it falls back to a locally generated per-install keyfile so secrets are still encrypted by default. Memory content, OAuth tokens, and secret-backed integration credentials all use the same encrypted storage path. Secrets entered through the built-in secret flows (for example `set secret ...`, integration settings, or runtime `{{secret:KEY}}` placeholders) are stored encrypted and resolved at execution time, so they do not appear in normal LLM-visible tool-call arguments or traces. If a user pastes a secret directly into ordinary chat, that message is still chat content and should be treated as exposed to the model.

### Talk to it like this:

```
> Monitor Hacker News every 30 minutes and notify me on Telegram if anything
  about "AI agents" hits the front page.

> Every morning at 9am, check my calendar and give me a briefing with
  weather, top news, and any tasks I'm behind on.

> Build me a landing page for my new project. Deploy it with a public URL.

> Search the web for recent papers on multi-agent architectures,
  summarize the top 3, and save them to my documents.

> Remember that I prefer concise answers, hate bullet-point lists,
  and my timezone is EST.

> Post on Moltbook about what I've been building this week.
  Keep the tone casual but technical.
```

It doesn't just respond - it **schedules the watcher**, **deploys the app**, **saves to memory**, **posts the content**. Then it follows up tomorrow.

### What it actually does:

| | |
|:--|:--|
| **Thinks** | Parallel reasoning, strategy selection, multi-step planning |
| **Acts** | Sandboxed tool execution - web search, file ops, app deployment, API calls |
| **Remembers** | Persistent memory across every conversation, channel, and restart |
| **Monitors** | Background watchers that poll conditions and alert you when they trigger |
| **Schedules** | Cron tasks, recurring goals, autonomous routines that run unattended |
| **Deploys** | Builds apps from a prompt and exposes them through a Cloudflare tunnel |
| **Evolves** | Self-improving - rewrites its own prompts and strategies based on outcomes |
| **Connects** | One agent reachable from web UI, CLI, Telegram, and WhatsApp |

### How it stacks up:

|                    | AgentArk | OpenClaw | NanoBot | PicoClaw | ZeroClaw |
|:-------------------|:---------|:---------|:--------|:---------|:---------|
| **Cold start**     | **48ms** | 1200ms   | 800ms   | 90ms     | 60ms     |
| **Idle RAM**       | **34MB** | 210MB    | 85MB    | 18MB     | 12MB     |
| **Binary**         | 38MB     | 120MB    | 45MB    | 12MB     | 8MB      |
| **Language**       | Rust     | TypeScript | Python | Go       | Rust     |
| **Memory system**  | 3-tier   | None     | Basic   | None     | None     |
| **WASM sandbox**   | Yes      | No       | No      | No       | No       |
| **Self-evolution** | Yes      | No       | No      | No       | No       |

> AgentArk is heavier than minimal agents because it bundles a full web UI, WASM sandbox, Playwright browser automation, and a 3-tier memory system. The others are lighter because they do less. [Full benchmark details below.](#benchmark-snapshot)

## The Idea

Every AI tool today asks you to choose:

| | Good at | Bad at |
|:--|:--|:--|
| **ChatGPT / Claude** | Answering questions | Remembering you. Doing things. Following up. |
| **Cursor / Copilot** | Writing code | Everything outside the editor |
| **Zapier / n8n** | Explicit automations | Reasoning. Conversation. Adapting. |
| **Hosted AI agents** | Convenience | You don't own your data, runtime, or state |

**AgentArk refuses to choose.** It combines conversational AI, autonomous execution, persistent memory, and operational infrastructure into one system - and you run it yourself.

**Design principles:**

- **Chat-first** - talk to it naturally, not through config files or flowcharts
- **Memory is the default** - every interaction builds on every previous one
- **Security is structural** - WASM sandbox, action guards, AES-256-GCM encryption, and verifiable records of what the agent actually did
- **Always-on** - tasks, watchers, and goals run in the background even when you're not chatting
- **Model-agnostic** - OpenAI, Anthropic, Google, Ollama, or any OpenAI-compatible endpoint
- **Self-hosted** - your hardware, your data, your rules. Period.

In practice, execution proofs mean you can look back and answer user questions like: What did the agent run? What changed? Why did it fail? Can I trust this result enough to approve the next step?

## Install

### One-liner (Linux / macOS)

```bash
curl -sSL https://raw.githubusercontent.com/agentark-ai/AgentArk/main/scripts/install.sh | bash
```

Installs Docker if needed, pulls AgentArk, and starts everything.
Open **http://localhost:8990** when it's done.

### Docker Compose

```bash
git clone https://github.com/agentark-ai/AgentArk.git && cd AgentArk

# Start
docker compose up -d --build          # uses an external LLM (OpenRouter, Anthropic, OpenAI …)
docker compose --profile with-ollama up -d --build   # bundle a local Ollama instance
docker compose --profile with-search up -d --build   # bundle SearXNG private search

# Windows
scripts\start.bat
```

### Build from source

```bash
# Rust 1.75+
cargo build --release
./target/release/agentark --headless
```

### Remote access

Built-in Cloudflare tunnel, toggleable from the Settings page in the web UI. No ports to open, no signup, traffic encrypted end-to-end.

### Management

```bash
docker compose up -d --build                    # build and start
docker compose down                             # stop
docker compose logs -f agentark                 # follow logs
docker compose up -d --build --force-recreate   # rebuild and restart
```

### CLI Mode

Talk to your agent directly from the terminal - no browser needed:

```bash
agentark chat                    # start chatting
agentark pulse                   # run ArkPulse health check
```

```
╔═══════════════════════════════════════════════════════════╗
║           AgentArk v0.1.0 - CLI Chat                    ║
╠═══════════════════════════════════════════════════════════╣
║  Type your message and press Enter.                      ║
║  Commands: /exit  /new  /help                            ║
╚═══════════════════════════════════════════════════════════╝

you ➜ what can you do?
agentark ➜ I can help with...
```

CLI trace flow:

- `Ctrl+T` toggles trace mode on or off without leaving chat
- When trace mode is on, the full step-by-step trace prints before each agent reply
- The trace is styled separately from the normal reply, so diagnostics stay distinct from the final answer
- `Ctrl+D` exits the CLI chat cleanly
- `Tab` autocompletes slash commands like `/help`, `/new`, and `/exit`

What you can do from CLI:

| Command / Action | Example |
| --- | --- |
| Chat with the agent | Just type your message |
| Run ArkPulse health check | `run arkpulse` or `check system health` |
| Deploy & manage apps | `deploy a weather dashboard` |
| Read/write files | `read the config file` |
| Search the web | `search for latest AI news` |
| Manage tasks & goals | `show my tasks` |
| Send emails (with Gmail integration) | `send an email to ...` |
| Query documents | `what does the uploaded PDF say about ...` |
| Start new conversation | `/new` |
| Toggle full trace mode | `Ctrl+T` |
| Autocomplete slash commands | `Tab` |
| Exit quickly | `Ctrl+D` |
| Exit | `/exit` |

> All capabilities available in the Web UI work in CLI mode - the agent has the same tools, memory, and integrations.

`agentark pulse` is not chat wrapped. It runs a dedicated ArkPulse CLI path, prints the latest health snapshot directly, and avoids the chat banner / stdin quirks that came from piping a prompt into `--chat`.

### Upcoming: Self-Update

In-app self-update is planned, but it is currently disabled in the product.

- The UI entry points are hidden.
- The HTTP routes are unmounted.
- The updater worker is not started by Docker Compose or the helper scripts.
- The implementation code is still kept in the repo for a later return.

When it comes back, the intended flow is:

1. A user explicitly approves an update request from the app.
2. AgentArk writes an update job instead of updating itself inline.
3. A separate updater process picks up that job outside the main app process.
4. The updater validates the change, rebuilds the image/service, restarts it, and runs health checks.
5. If the new version fails health checks, the updater can roll back to the previous known-good version.

It is disabled for now to avoid accidental or confusing in-app upgrades for non-technical users. For the moment, use the manual update path above when you want to upgrade a deployment.

---

## High-Level Benchmark

This section is a product-shape comparison, not a synthetic benchmark. The low-level startup/RAM/cost snapshot is further below in [Benchmark Snapshot](#benchmark-snapshot).

| Compared with | Strongest at | Where AgentArk is different |
| --- | --- | --- |
| **ChatGPT** | General chat, research, broad consumer UX | AgentArk is self-hosted and built for persistent execution: memory, tasks, watchers, integrations, app deployment, and local control over secrets/runtime |
| **Claude Code** | Deep repo work from the terminal/IDE | AgentArk is broader than a coding agent: it adds autonomy modes, channels, goals, background automation, and non-dev operational workflows |
| **n8n** | Explicit workflow automation with strong integration coverage | AgentArk is chat-first and agent-first: less flowchart orchestration, more memory, planning, sub-agents, and ongoing autonomous behavior |
| **OpenHands** | AI-driven software engineering and developer workflows | AgentArk is aimed at a wider operating model: personal ops, business ops, incident handling, multi-channel interaction, and long-running agent state outside pure SWE tasks |

In short:

- choose **ChatGPT** if you mainly want the best hosted general assistant experience
- choose **Claude Code** if your main job is shipping code inside an existing repo
- choose **n8n** if you want deterministic workflow graphs and integration-heavy pipelines
- choose **OpenHands** if you want an open software-engineering agent
- choose **AgentArk** if you want one self-hosted agent that can chat, automate, monitor, remember, and operate over time

---

## Features

### Core

|                             |                                                                                                   |
| --------------------------- | ------------------------------------------------------------------------------------------------- |
| **Parallel Thinking**       | Multiple reasoning paths processed simultaneously - 25-35 % cost reduction                        |
| **Sub-Agent Orchestration** | Researcher · Coder · Analyst · Writer · Validator - auto-selected per task                        |
| **Self-Evolve Engine**      | Policy evolution, strategy tuning, and routing benchmarks that improve the agent over time        |
| **Self-Tune**               | Learns your style, tracks tool success rates, auto-adjusts autonomy confidence — adapts to you over time |
| **Cognitive Memory**        | Three-tier: Episodic (conversations) · Semantic (facts) · Procedural (actions) with decay scoring |
| **Live App Deployment**     | Deploy static or dynamic apps from chat - Node, Python, HTML, and more                            |
| **Goal Autopilot**          | Goal → plan → scheduled execution → recurring progress reports                                    |
| **Predictive Nudges**       | Early warnings for missed deadlines, overdue pressure, and recommended next actions               |

### Security

|                           |                                                                                                |
| ------------------------- | ---------------------------------------------------------------------------------------------- |
| **AES-256-GCM + Argon2**  | All secrets encrypted at rest; industry-standard key derivation                                |
| **Action Security Guard** | 4-pillar defense: integrity signing, static analysis, permissions, injection scanning          |
| **Prompt Protection**     | Injection detection, leakage prevention, output redaction                                      |
| **Sandboxed Execution**   | WASM (Wasmtime) + Docker isolation with automatic rollback                                     |
| **Execution Proofs**      | Verifiable records of what the agent actually did, useful for trust, debugging, approvals, and audits |
| **10-Layer Hardening**    | API key auth, localhost bind, CORS, rate limiting, Docker socket proxy, optional TLS, and more |

### Integrations

|                   |                                                                                                          |
| ----------------- | -------------------------------------------------------------------------------------------------------- |
| **Channels**      | Telegram · WhatsApp (Baileys + Cloud API) · Web UI                                                       |
| **LLM Providers** | Ollama · Anthropic · OpenAI · OpenRouter · any OpenAI-compatible API                                     |
| **Connectors**    | GitHub · Notion · Twitter/X · Google Places · 1Password · Twilio · Shopify                               |
| **MCP Servers**   | HTTP JSON-RPC + stdio transports, hot-reload, encrypted credentials                                      |
| **Media**         | Image gen (DALL-E, Stability, Fal, Replicate) · Video gen (Runway, Luma) · Audio transcription (Whisper) |
| **Utilities**     | PDF generation · Expense tracking · Invoice creation · Daily briefing · Weekly review                    |

### Autonomy Control Plane

Policy-driven proactive operation with enterprise guardrails:

- **Daily Command Brief** - risks, opportunities, and 3 executable recommendations at login
- **Autopilot Modes** - `Focus` · `Ops` · `Travel` · `Finance` - declarative routines + watchers
- **Smart Inbox Triage** - auto-clusters messages: Act now / Delegate / Ignore
- **Live Incident Copilot** - executable containment/recovery playbooks
- **Cross-Channel Continuity** - configurable `per_channel` or `global` context scope
- **Outcome Timeline + Rollback** - replayable event timeline with safe rollback operations
- **Trust Layer** - risk scoring, policy-based blocking, approval escalation
- **One-Click Delegation Swarm** - delegate strategic tasks to specialist sub-agents

---

## Configuration

### First-time setup

1. Open **http://localhost:8990**
2. Go to **Settings** (gear icon)
3. Pick your **LLM Provider** and enter credentials
4. Set **Bot Name** and **Personality**
5. Save → start chatting

### LLM providers

| Provider              | Base URL                       | Example models                        |
| --------------------- | ------------------------------ | ------------------------------------- |
| **Ollama** (local)    | `http://localhost:11434`       | `llama3.2`, `qwen2.5`, `mistral`      |
| **OpenRouter**        | `https://openrouter.ai/api/v1` | `glm-4`, `qwen/qwen-2.5-72b-instruct` |
| **Anthropic**         | built-in                       | `claude-sonnet-4-20250514`            |
| **OpenAI**            | built-in                       | `gpt-4o`, `gpt-4-turbo`               |
| **OpenAI-compatible** | your URL                       | any compatible model                  |

### Telegram bot (optional)

1. Create a bot via [@BotFather](https://t.me/BotFather)
2. Enable Telegram in Settings, paste the token
3. Add your user ID to Allowed Users (get it from [@userinfobot](https://t.me/userinfobot))
4. Save → Restart Bot

### Config files

```
config/
├── config.toml      # main config (non-sensitive)
├── secrets.enc      # encrypted API keys and tokens
└── .keyfile         # encryption key (auto-generated)
```

### Environment variables

| Variable          | Default          | Description                                  |
| ----------------- | ---------------- | -------------------------------------------- |
| `AGENTARK_CONFIG` | `/app/config`    | Configuration directory                      |
| `AGENTARK_DATA`   | `/app/data`      | Data directory                               |
| `AGENTARK_BIND`   | `127.0.0.1:8990` | HTTP bind address                            |
| `AGENTARK_DEBUG`  | `false`          | Enable debug logging                         |
| `TUNNEL_TOKEN`    | _(empty)_        | Cloudflare Tunnel token for permanent domain |
| `RUST_LOG`        | `info`           | Log level (`debug`, `info`, `warn`, `error`) |

---

## API

Full interactive API docs available at **http://localhost:8990/docs#/** after starting AgentArk.

---

## Why Rust?

|                     |                                                                                   |
| ------------------- | --------------------------------------------------------------------------------- |
| **Performance**     | Tokio async runtime, `Arc<RwLock<T>>` concurrency - no GIL bottleneck             |
| **Security**        | `Zeroizing` auto-clears secrets from memory; zero `unsafe` blocks in the codebase |
| **Type Safety**     | Enums, traits, and compile-time guarantees catch bugs before production           |
| **Single Binary**   | One compiled binary + Docker - no dependency hell                                 |
| **WASM Sandboxing** | Wasmtime integration is natural in Rust; awkward in interpreted languages         |

### Benchmark Snapshot

Local machine quick benchmark (macOS arm64, Feb 2026) - normalized for 0.8 GHz edge hardware.

|                       | **OpenClaw**  | **NanoBot**    | **PicoClaw** | **ZeroClaw** | **AgentArk** |
| --------------------- | ------------- | -------------- | ------------ | ------------ | ------------ |
| **Language**          | TypeScript    | Python         | Go           | Rust         | Rust         |
| **RAM**               | > 1 GB        | > 100 MB       | < 10 MB      | < 5 MB       | ~34 MB       |
| **Startup (0.8 GHz)** | > 500 s       | > 30 s         | < 1 s        | < 10 ms      | < 50 ms      |
| **Binary Size**       | ~28 MB (dist) | N/A (scripts)  | ~8 MB        | ~8.8 MB      | ~56 MB       |
| **Cost**              | Mac Mini $599 | Linux SBC ~$50 | Linux $10    | Any hardware | Any hardware |

> **Notes:** All results are bare-metal, no container overhead. OpenClaw requires Node.js runtime (~390 MB additional memory overhead), NanoBot requires Python runtime. PicoClaw, ZeroClaw, and AgentArk are static binaries. AgentArk's larger binary includes WASM sandbox (Wasmtime), Playwright browser automation, and a full web UI - features the others don't bundle. RAM figures are runtime memory; build-time requirements are higher.

### Monthly Cost Comparison

All platforms are free/open-source - the real cost is **AI tokens + hosting**.

|                    | OpenClaw             | NanoClaw      | TinyClaw                | **AgentArk**           |
| ------------------ | -------------------- | ------------- | ----------------------- | ---------------------- |
| Software           | Free                 | Free          | Free (or $30/mo hosted) | Free                   |
| Avg personal use   | $15 – $60/mo         | $5 – $50/mo   | $10 – $40/mo            | **$2 – $10/mo**        |
| Avg small team     | $40 – $120/mo        | $20 – $80/mo  | $30 – $70/mo            | **$10 – $30/mo**       |
| Heavy automation   | $100 – $400+/mo      | $50 – $150/mo | $50 – $120/mo           | **$20 – $50/mo**       |
| Risk of bill shock | High (GPT-4o/Claude) | Medium        | Medium (Claude CLI)     | **Low (cheap models)** |

> **User-reported OpenClaw horror stories:** \$47 burned in a single week of testing, \$50 in the first few days from badly configured cron jobs, and one developer hit **\$623/month** from runaway agent API usage.

**AgentArk's edge: cheap models + built-in security.** Route to DeepSeek, GLM, Mistral, or local Ollama for \$0.10–\$0.50/1M tokens vs \$3–\$15/1M on premium models - while still getting verifiable execution history, PII redaction, WASM sandboxing, and action-level approval gates that most alternatives lack entirely.

---

## Troubleshooting

<details>
<summary>Settings won't save</summary>

- Check that you have a valid API key for non-Ollama providers
- Ensure the model name is correct
</details>

<details>
<summary>Telegram bot not responding</summary>

- Restart after changing Telegram settings
- Verify your user ID is in Allowed Users
- Check bot token is correct
</details>

<details>
<summary>Data lost after restart</summary>

- Always use Docker volumes - `docker compose` and `scripts/start.sh` handle this automatically
- If using `docker run`, add `-v agentark-data:/app/data -v agentark-config:/app/config`
</details>

<details>
<summary>Debug logging</summary>

```bash
AGENTARK_DEBUG=true docker compose up        # full debug
RUST_LOG=info,agentark=debug docker compose up   # agent internals only
```

</details>

---

## Contributing

Contributions welcome! Here's how to get started:

### Setup

```bash
git clone https://github.com/agentark-ai/AgentArk.git && cd AgentArk

# Backend (Rust 1.75+)
cargo build                    # debug build
cargo test                     # run tests

# Frontend (Node 20+)
cd frontend && npm install && npm run dev   # dev server with hot reload

# Full stack via Docker
docker compose up -d --build
```

### Project structure

```
src/
├── core/           # Agent engine, LLM routing, memory, pipeline, self-evolve
├── actions/        # Tool implementations (SSH, search, apps, research)
├── channels/       # HTTP API, Telegram, WhatsApp
├── security/       # Action guard, safety rules
├── runtime/        # WASM + Docker sandboxing
├── storage/        # SQLite persistence, entities
├── integrations/   # GitHub, Notion, Twitter, MCP, etc.
└── main.rs         # Entrypoint

frontend/src/       # React + MUI + TypeScript web UI
config/             # Default config templates
skills/             # Built-in skill definitions
```

### Guidelines

- **PRs over issues** - code speaks louder than feature requests
- **One concern per PR** - keep changes focused and reviewable
- **Tests for new features** - add to `tests/` when adding functionality
- **No secrets in code** - use `SecureConfigManager` for anything sensitive
- **Format before push** - `cargo fmt` and `cd frontend && npx prettier --write src/`

## Acknowledgments

AgentArk is built on the shoulders of outstanding open-source projects:

| Project | Used for |
| --- | --- |
| [Rust](https://www.rust-lang.org/) | Core runtime — memory safety, performance, and fearless concurrency |
| [Tokio](https://tokio.rs/) | Async runtime powering all concurrent operations |
| [Axum](https://github.com/tokio-rs/axum) | HTTP server and API framework |
| [SeaORM](https://www.sea-ql.org/SeaORM/) | Database ORM over SQLite |
| [React](https://react.dev/) + [MUI](https://mui.com/) | Web UI framework and component library |
| [Playwright](https://playwright.dev/) | Browser automation for screenshots and complex SPA interaction |
| [Lightpanda](https://github.com/lightpanda-io/browser) | Fast headless browser for content extraction and web scraping |
| [Mem0](https://github.com/mem0ai/mem0) | Semantic memory layer with vector search and decay |
| [Cloudflared](https://github.com/cloudflare/cloudflared) | Zero-config tunnels for remote access |
| [ECharts](https://echarts.apache.org/) | Analytics charts and data visualization |
| [Wasmtime](https://wasmtime.dev/) | WebAssembly sandbox for secure code execution |
| [Bollard](https://github.com/fussybeaver/bollard) | Docker API client for container management |
| [Russh](https://github.com/warp-tech/russh) | SSH client for remote server operations |
| [react-markdown](https://github.com/remarkjs/react-markdown) | Markdown rendering in chat |
| [Teloxide](https://github.com/teloxide/teloxide) | Telegram bot framework |

Thank you to every contributor and maintainer of these projects.

## License

MIT OR Apache-2.0

---

<p align="center">
  Built with Rust 🦀
</p>
