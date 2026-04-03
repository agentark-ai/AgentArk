<p align="center">
  <img src="assets/logo.svg" alt="AgentArk Logo" width="420" height="240">
</p>

<h1 align="center">AgentArk 🚀</h1>


<p align="center">
  <em>Your secure daily AI assistant</em>
</p>

<p align="center">
  <strong>Private by default. Runs on your machine. Remembers what matters. Learns how you work. Delivers a daily brief. Asks before risky actions.</strong><br>
  Starts in &lt;50ms | ~34MB RAM | Runs on any hardware
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
  <a href="#what-is-agentark">What is it?</a> |
  <a href="#the-idea">Idea</a> |
  <a href="#install">Install</a> |
  <a href="#high-level-benchmark">Compare</a> |
  <a href="#architecture">Architecture</a> |
  <a href="#troubleshooting">Troubleshoot</a> |
  <a href="#why-rust">Why Rust?</a> |
  <a href="#benchmark-snapshot">Benchmarks</a> |
  <a href="#contributing">Contribute</a>
</p>

<p align="center">
  <strong>Daily brief | personal memory | self-evolves from use | encrypted secrets | safe actions | power automations when you want them</strong><br>
  Self-host anywhere. Connect any LLM. Stay private.
</p>

<p align="center">
  <code>AES-256-GCM encryption | Argon2id master-password mode | approvals | WASM sandbox | verifiable action history</code>
</p>

---

## What Is AgentArk?

AgentArk is a self-hosted personal AI assistant for daily life and work.

It runs on your machine, keeps track of your preferences, delivers a daily brief, follows up across channels, and can take action safely when you ask. When you need more than a chat app, it can also schedule routines, monitor things in the background, build apps, and run deeper self-improving automations.

It is built to evolve with you. AgentArk learns from accepted work, user corrections, repeated routines, and live tool outcomes so the assistant gets more aligned with your workflow instead of acting like every session is day one.

Short examples:

- If you keep rewriting replies to be shorter, it learns to stay concise by default
- If a certain tool path keeps succeeding for a task, it becomes more likely to choose that path again
- If you correct how it briefs, routes, or follows up, future runs can reflect that correction instead of repeating the same mistake

Your data stays with you. Your secrets are encrypted. You keep the final say on risky actions.

### Why AgentArk feels different

Most AI assistants force a bad tradeoff: easy but forgetful, or powerful but too exposed. AgentArk is built to stay useful every day without giving up control.

| Category | Typical hosted or lightweight assistants | AgentArk |
|:--|:--|:--|
| **Privacy** | State usually lives in a vendor account or cloud service | Self-hosted, local-first, encrypted secrets, and configurable access controls |
| **Daily usefulness** | Good at one-off chats, weak at follow-up | Memory, recurring briefs, reminders, messaging channels, and durable tasks |
| **Trust model** | Easy to ask, harder to verify | Approvals, security logs, execution history, and guarded actions |
| **Continuity** | Context often resets every session | Durable user facts, preferences, documents, task state, and integration state |
| **Improvement loop** | Behavior stays mostly static unless manually reprompted or reconfigured | Self-tune and self-evolve learn from accepted work, corrections, tool outcomes, and benchmarks |
| **Power features** | Automation is shallow or separated from chat | Tasks, watchers, apps, integrations, swarm agents, and reusable skills |
| **Security model** | Secrets and actions are often bolted on around the prompt | AES-256-GCM at rest, Argon2id master-password mode, sandboxing, approvals, and verifiable history |

Security is not just a checkbox here. Sensitive config is split from readable config: normal settings live in `config.toml`, while API keys, OAuth tokens, and custom secrets are stored encrypted in `secrets.enc`. When a master password is set, AgentArk derives the encryption key with **Argon2id** and uses **AES-256-GCM** for encryption at rest. Without a master password, it falls back to a locally generated per-install keyfile so secrets are still encrypted by default. Memory content, OAuth tokens, and secret-backed integration credentials all use the same encrypted storage path. Secrets entered through the built-in secret flows, such as `set secret ...`, integration settings, or runtime `{{secret:KEY}}` placeholders, are stored encrypted and resolved at execution time, so they do not appear in normal LLM-visible tool-call arguments or traces.

### Talk to it like this:

```
> Every weekday at 9am, send me a daily brief with weather,
  calendar, urgent email, and overdue tasks.

> Remember that I prefer concise answers and daily updates in Telegram.

> Watch my inbox for urgent client messages and alert me if I do not reply.

> Draft a reply to this message and ask before sending it.

> Build me a landing page for my new project. Deploy it with a public URL.

> Search the web for recent papers on multi-agent architectures,
  summarize the top 3, and save them to my documents.
```

It does not stop at a reply. It can **save the preference**, **schedule the follow-up**, **deliver the brief**, **draft the reply**, **watch for updates**, or **promote the work into a durable task** and come back later.

### What it actually does:

| | |
|:--|:--|
| **Keeps up with you** | Persistent memory across every conversation, channel, and restart |
| **Briefs you daily** | Calendar, weather, tasks, alerts, and messaging-channel delivery |
| **Acts safely** | Sandboxed tool execution, approvals, guarded actions, and security logs |
| **Follows up** | Background watchers, scheduled tasks, and routines that run unattended |
| **Reaches you** | One assistant reachable from web UI, CLI, Telegram, and WhatsApp |
| **Researches and builds** | Web search, file work, app deployment, API calls, and grounded summaries |
| **Improves over time** | Self-tune and self-evolve adapt prompts, routing, and strategy from corrections and live outcomes |
| **Grows with power users** | Apps, integrations, swarm agents, reusable skills, and self-improving autonomy features |

### How it stacks up:

|                    | AgentArk | NanoBot | PicoClaw | ZeroClaw |
|:-------------------|:---------|:--------|:---------|:---------|
| **Cold start**     | **48ms** | 800ms   | 90ms     | 60ms     |
| **Idle RAM**       | **34MB** | 85MB    | 18MB     | 12MB     |
| **Binary**         | 38MB     | 45MB    | 12MB     | 8MB      |
| **Language**       | Rust     | Python  | Go       | Rust     |
| **Memory system**  | 3-tier   | Basic   | None     | None     |
| **WASM sandbox**   | Yes      | No      | No       | No       |
| **Self-evolution** | Yes      | No      | No       | No       |

> AgentArk is heavier than minimal agents because it bundles a full web UI, WASM sandbox, Playwright browser automation, and a 3-tier memory system. The others are lighter because they do less. [Full benchmark details below.](#benchmark-snapshot)

## The Idea

Most self-hosted agents, whether OpenClaw, PicoClaw, NanoClaw, Agent Zero, or similar projects, still force a tradeoff:

| | Good at | Tradeoff |
|:--|:--|:--|
| **OpenClaw** | Fast local execution and lightweight setup | Less durable personal memory, follow-up, and day-to-day assistant continuity |
| **PicoClaw** | Small footprint and low-overhead agent runtime | Less depth across channels, guarded actions, and longer workflows |
| **NanoClaw** | Minimal runtime and simple local operation | Less compounding adaptation, background routines, and self-improvement |
| **Agent Zero** | Open-ended autonomy and experimentation | Less opinionated trust, personal-assistant polish, and compact daily usability |

**AgentArk refuses to choose.** It gives you a private assistant that is useful every day, plus deeper automation when you want it, and you still own the data, runtime, and state.

**Design principles:**

- **Secure first** - encrypted secrets, approvals, sandboxing, and verifiable records of what the assistant did
- **Daily by default** - briefs, reminders, follow-up, and messaging delivery are first-class workflows
- **Memory that compounds** - each useful interaction can build on previous preferences, facts, and documents
- **Self-evolving** - accepted work, user corrections, tool outcomes, and benchmarks improve prompts, routing, and strategy over time
- **Chat-first** - talk to it naturally, not through config files or flowcharts
- **Power when needed** - tasks, watchers, apps, integrations, and swarm features stay available for deeper work
- **Model-agnostic** - OpenAI, Anthropic, Google, Ollama, or any OpenAI-compatible endpoint
- **Self-hosted** - your hardware, your data, your rules

In practice, execution proofs mean you can look back and answer questions like: What did the assistant run? What changed? Why did it fail? Can I trust this enough to approve the next step?

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

# Start AgentArk with bundled Postgres
docker compose up -d --build

# Reset everything, including the Postgres data volume
docker compose down -v

# Windows
scripts\start.bat
```

This starts AgentArk with the bundled database and default local setup. Most installs do not need extra `.env` work: open `http://localhost:8990`, choose your model in Settings, and start chatting.

### Container variants

AgentArk publishes two GHCR variants for deployments: a lean `base` image and a heavier `full` image.

| Variant | What it includes | Approx. size (linux/amd64) |
|:--|:--|:--|
| `ghcr.io/agentark-ai/agentark:base` | Core AgentArk server, web UI, Git, Python app runtime | ~900 MB |
| `ghcr.io/agentark-ai/agentark:full` | Base image plus Playwright/Chromium, cloudflared, tailscale, Lightpanda, Google Workspace CLI, bundled WhatsApp bridge for on-demand Baileys mode | ~4.5 GB |

Pull whichever image matches your deployment:

```bash
docker pull ghcr.io/agentark-ai/agentark:base
docker pull ghcr.io/agentark-ai/agentark:full
```

Release publishes also create versioned tags in the same shape, for example:

```bash
docker pull ghcr.io/agentark-ai/agentark:1.2.3-base
docker pull ghcr.io/agentark-ai/agentark:1.2.3-full
```

Tag behavior:
- pushes to `main` refresh moving tags like `:base`, `:full`, `:latest-base`, and `:latest-full`
- `v*` releases publish those moving tags and the matching versioned tags like `:1.2.3-base` and `:1.2.3-full`

### Local builds

Local `docker compose build` always produces the **full** image (~12.5 GB) with all runtimes included:

```bash
docker compose up -d --build
```

Compose-managed installs reuse the installed AgentArk image as the default runtime/app image. The default path does not expect a separate `agentark-sandbox` image; only set `AGENTARK_RUNTIME_IMAGE` if you intentionally want a different runner image.

The slim base image (~900 MB) is only available via GHCR pulls. If you want a lighter local build, use build args to disable specific components:

```bash
docker compose build --build-arg INSTALL_OLLAMA_CLI=false
docker compose up -d --force-recreate
```

The Docker build defaults to `2` Rust compile jobs. On stronger machines you can raise it with `docker compose build --build-arg AGENTARK_BUILD_JOBS=4`, or use `AGENTARK_BUILD_JOBS=0` to let Cargo choose its default parallelism.

### Build from source

```bash
# Rust 1.75+
export AGENTARK_DATABASE_URL=postgres://agentark:agentark@localhost:5432/agentark
export AGENTARK_DB_MAX_CONNECTIONS=20
export AGENTARK_DB_CONNECT_TIMEOUT_SECS=5
export AGENTARK_DB_STATEMENT_TIMEOUT_MS=30000
export AGENTARK_DB_IDLE_TIMEOUT_SECS=300
cargo build --release
./target/release/agentark --headless
```

### Remote access

Built-in remote access is toggleable from the Settings page in the web UI. Cloudflare Quick Tunnel is the default public-link mode: no ports to open, no signup, traffic encrypted in transit. For private end-to-end encrypted access, choose Tailscale private access from Settings and use your tailnet devices.

### Management

Updates are manual today. Rebuild and restart source-based installs with the commands below, or use the installer-provided `agentark update` wrapper on managed installs.

```bash
docker compose up -d --build                    # build and start
docker compose down                             # stop
docker compose down -v                           # stop and reset Postgres + volumes
docker compose logs -f agentark                 # follow logs
docker compose up -d --build --force-recreate   # rebuild and restart
```

### CLI Mode

Talk to your agent directly from the terminal - no browser needed:

```bash
agentark chat                    # start chatting
agentark setup                   # guided CLI setup wizard
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

If no chat model is configured yet, `agentark chat` now stops before opening the banner, shows the missing setup steps, and offers to launch `agentark setup` immediately in interactive terminals.

`agentark pulse` is not chat wrapped. It runs a dedicated ArkPulse CLI path, prints the latest health snapshot directly, and avoids the chat banner / stdin quirks that came from piping a prompt into `--chat`.

### External Launchers (Optional)

AgentArk can manage **optional external launchers** in the **Apps** view through Ollama Launch. These are companion tools that AgentArk can prepare or invoke. They are **not** AgentArk modes or rebrands.

- Claude Code
- Codex
- OpenCode

These tools are still **terminal-first**. AgentArk can generate the exact commands, show runtime readiness, and attempt a server-side launch, but the full interactive experience is still best in your own terminal.

Optional external tool commands:

```bash
# Docker-hosted AgentArk: run from your host terminal
docker exec -it -e OLLAMA_HOST=http://host.docker.internal:11434 agentark ollama launch claude
docker exec -it -e OLLAMA_HOST=http://host.docker.internal:11434 agentark ollama launch claude --model minimax-m2.5:cloud
docker exec -it -e OLLAMA_HOST=http://host.docker.internal:11434 agentark ollama launch claude --config

docker exec -it -e OLLAMA_HOST=http://host.docker.internal:11434 agentark ollama launch codex
docker exec -it -e OLLAMA_HOST=http://host.docker.internal:11434 agentark ollama launch codex --model gpt-oss:120b
docker exec -it -e OLLAMA_HOST=http://host.docker.internal:11434 agentark ollama launch codex --config

docker exec -it -e OLLAMA_HOST=http://host.docker.internal:11434 agentark ollama launch opencode
docker exec -it -e OLLAMA_HOST=http://host.docker.internal:11434 agentark ollama launch opencode --model qwen3.5:cloud
docker exec -it -e OLLAMA_HOST=http://host.docker.internal:11434 agentark ollama launch opencode --config

```

If AgentArk is running directly on the host instead of Docker, use the in-runtime commands:

```bash
ollama launch claude
ollama launch codex
ollama launch opencode
```

Notes:

- The Apps → External Launchers panel uses your configured Ollama base URL from AgentArk settings.
- If AgentArk is running in Docker and Ollama is running on your host machine, use `http://host.docker.internal:11434` instead of `http://localhost:11434`.
- Ollama is optional and user-managed. AgentArk does not bundle or start an Ollama service for you.
- OpenCode works best with models that support **64K context or more**.

---

## High-Level Benchmark

This section is a product-shape comparison against other self-hosted agents, not a synthetic benchmark. The low-level startup/RAM/cost snapshot is further below in [Benchmark Snapshot](#benchmark-snapshot).

| Compared with | Strongest at | Where AgentArk is different |
| --- | --- | --- |
| **OpenClaw** | Fast local execution and lightweight setup | AgentArk adds daily-assistant continuity, durable memory, guarded actions, and self-evolution from use |
| **PicoClaw** | Small-footprint runtime and low-overhead operation | AgentArk adds multi-channel workflows, deeper automation, richer memory, and adaptive improvement over time |
| **NanoClaw** | Minimal local operation and simplicity | AgentArk adds background routines, stronger follow-up, broader task execution, and compounding learning loops |
| **Agent Zero** | Open-ended autonomy and experimentation | AgentArk adds a stronger trust layer, personal-assistant UX, and a more opinionated self-evolve control plane |

In short:

- choose **OpenClaw** if you want a lightweight local agent runtime first
- choose **PicoClaw** if your top priority is a smaller-footprint agent
- choose **NanoClaw** if you want the most minimal local agent setup
- choose **Agent Zero** if you want open-ended autonomous experimentation
- choose **AgentArk** if you want one self-hosted agent that can chat, automate, remember, follow up, and improve over time

---

## Features

### Core

|                             |                                                                                                   |
| --------------------------- | ------------------------------------------------------------------------------------------------- |
| **Parallel Thinking**       | Multiple reasoning paths processed simultaneously - 25-35 % cost reduction                        |
| **Sub-Agent Orchestration** | Researcher · Coder · Analyst · Writer · Validator - auto-selected per task                        |
| **Self-Evolve Engine**      | Prompt evolution, policy tuning, strategy learning, and routing benchmarks that improve the agent over time |
| **Self-Tune**               | Learns your style, tracks tool success rates, auto-adjusts autonomy confidence — adapts to you over time |
| **Cognitive Memory**        | Three-tier: Episodic (conversations) · Semantic (facts) · Procedural (actions) with decay scoring |
| **Live App Deployment**     | Deploy static or dynamic apps from chat - Node, Python, HTML, and more                            |
| **Goal Autopilot**          | Goal → plan → scheduled execution → recurring progress reports                                    |
| **Predictive Nudges**       | Early warnings for missed deadlines, overdue pressure, and recommended next actions               |
| **Background Learning**     | Periodic reflection pass, memory consolidation, and pattern induction that improve suggestions and briefs |

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
| **Channels**      | Telegram · WhatsApp (Baileys with bundled or external bridge + Cloud API) · Web UI                     |
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

### Background Learning

AgentArk also runs a background learning loop inside Sentinel. It periodically reflects on recent activity, consolidates memory, and folds repeated successful patterns back into the system.

What users get:

- Better suggestions from recent runs and repeated patterns
- Cleaner memory with fewer duplicates and stale entries
- Improved briefings that stay aligned with recent activity
- Stronger follow-up on recurring tasks and reminders

In the Sentinel panel, this appears as `Background learning` with sub-categories for reflection pass, memory consolidation, experience consolidation, pattern induction, and candidate generation. These are operational background jobs surfaced as runtime status, not a separate assistant or branded mode.

Users can also ask AgentArk in chat to check current background learning status, explain why a job is paused or failing, and walk through the live Sentinel state for debugging.

For unattended runs, AgentArk also fails closed on missing critical inputs instead of guessing. If a scheduled or background task cannot continue safely, it moves to `Input needed`, emits a notification, and shows the exact missing fields in Tasks and Trace so the user can fix them and resume.

---

## Configuration

### First-time setup

Web UI flow:

1. Open **http://localhost:8990**
2. Go to **Settings** (gear icon)
3. Pick your **LLM Provider** and enter credentials
4. Set **Bot Name** and **Personality**
5. Save → start chatting

CLI-first flow:

1. Run `agentark setup`
2. Pick your chat model/provider and optional Telegram settings
3. Run `agentark chat`

If you skip setup and run `agentark chat` first, AgentArk will explain what is missing and offer to launch the CLI setup wizard automatically.

### Default stack notes

- Docker Compose starts Postgres and AgentArk's internal services automatically. Normal installs do not need extra service-token setup.
- Local embeddings are the default and use the built-in Hugging Face path with `sentence-transformers/all-MiniLM-L6-v2`.
- In `Settings > Models > Embeddings`, `External` supports user-managed OpenAI-compatible embedding endpoints, including Ollama if you run it yourself.
- The Docker image includes bundled skills under `/app/skills`. Deleting one in the Skills UI removes it for that install; fresh installs restore the bundled defaults from the image.
- Compose sets `AGENTARK_DATABASE_URL` automatically for the app container. Native binary installs must provide their own Postgres URL.
- Optional Postgres tuning env vars for native and container runs: `AGENTARK_DB_MAX_CONNECTIONS`, `AGENTARK_DB_CONNECT_TIMEOUT_SECS`, `AGENTARK_DB_STATEMENT_TIMEOUT_MS`, `AGENTARK_DB_IDLE_TIMEOUT_SECS`, and `AGENTARK_DB_SCHEMA`.
- Internally, the default deployment uses separate control, executor, and workspace services, but normal users still start everything with one command and open a single app at `http://localhost:8990`.

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
| `TUNNEL_TOKEN`    | _(empty)_        | Legacy Cloudflare Tunnel token for permanent domain |
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

|                       | **NanoBot**    | **PicoClaw** | **ZeroClaw** | **AgentArk** |
| --------------------- | -------------- | ------------ | ------------ | ------------ |
| **Language**          | Python         | Go           | Rust         | Rust         |
| **RAM**               | > 100 MB       | < 10 MB      | < 5 MB       | ~34 MB       |
| **Startup (0.8 GHz)** | > 30 s         | < 1 s        | < 10 ms      | < 50 ms      |
| **Binary Size**       | N/A (scripts)  | ~8 MB        | ~8.8 MB      | ~56 MB       |
| **Cost**              | Linux SBC ~$50 | Linux $10    | Any hardware | Any hardware |

> **Notes:** All results are bare-metal, no container overhead. NanoBot requires Python runtime. PicoClaw, ZeroClaw, and AgentArk are static binaries. AgentArk's larger binary includes WASM sandbox (Wasmtime), Playwright browser automation, and a full web UI - features the others don't bundle. RAM figures are runtime memory; build-time requirements are higher.

### Monthly Cost Comparison

All platforms are free/open-source - the real cost is **AI tokens + hosting**.

|                    | NanoClaw      | TinyClaw                | **AgentArk**           |
| ------------------ | ------------- | ----------------------- | ---------------------- |
| Software           | Free          | Free (or $30/mo hosted) | Free                   |
| Avg personal use   | $5 - $50/mo   | $10 - $40/mo            | **$2 - $10/mo**        |
| Avg small team     | $20 - $80/mo  | $30 - $70/mo            | **$10 - $30/mo**       |
| Heavy automation   | $50 - $150/mo | $50 - $120/mo           | **$20 - $50/mo**       |
| Risk of bill shock | Medium        | Medium (premium hosted models) | **Low (cheap models)** |

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
├── storage/        # PostgreSQL persistence, entities
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
| [SeaORM](https://www.sea-ql.org/SeaORM/) | Database ORM over PostgreSQL |
| [React](https://react.dev/) + [MUI](https://mui.com/) | Web UI framework and component library |
| [Playwright](https://playwright.dev/) | Browser automation for screenshots and complex SPA interaction |
| [Lightpanda](https://github.com/lightpanda-io/browser) | Fast headless browser for content extraction and web scraping |
| [Cloudflared](https://github.com/cloudflare/cloudflared) | Default public-link remote access via Cloudflare Tunnel |
| [Tailscale](https://tailscale.com/) | Private tailnet access with end-to-end encryption |
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
