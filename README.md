<p align="center">
  <img src="assets/banner.png" alt="AgentArk" width="700">
</p>

<p align="center">
  <em>Personal AI Assistant — Self-hosted, private, always learning</em>
</p>

<p align="center">
  <strong>Your AI. Your data. Your ark.</strong>
</p>

<p align="center">
  <a href="#install"><img src="https://img.shields.io/badge/INSTALL-Docker_Compose-2ea44f?style=for-the-badge" alt="Install"></a>
  <a href="#what-is-agentark"><img src="https://img.shields.io/badge/WEB_UI-localhost:8990-7C3AED?style=for-the-badge" alt="Web UI"></a>
  <a href="#license"><img src="https://img.shields.io/badge/LICENSE-MIT_%2F_Apache--2.0-orange?style=for-the-badge" alt="License"></a>
  <a href="#why-rust"><img src="https://img.shields.io/badge/RUST-250K_lines-B7410E?style=for-the-badge&logo=rust&logoColor=white" alt="Rust"></a>
</p>

<p align="center">
  A self-hosted assistant that remembers, follows up, and improves over time.<br>
  Private by default. Runs on your machine. Asks before risky actions.<br>
  <code>&lt;50ms cold start &middot; ~34MB RAM &middot; AES-256-GCM encrypted &middot; model-agnostic</code>
</p>

<p align="center">
  <a href="#install">Install</a> &middot;
  <a href="#features">Features</a> &middot;
  <a href="#configuration">Configuration</a> &middot;
  <a href="#architecture">Architecture</a> &middot;
  <a href="#security">Security</a> &middot;
  <a href="#api">API</a> &middot;
  <a href="#contributing">Contributing</a>
</p>

---

### Talk to it like this

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

---

## What Is AgentArk?

AgentArk is a self-hosted personal AI assistant for daily life and work.

It runs on your machine, keeps track of your preferences, delivers a daily brief, follows up across channels, and can take action safely when you ask. When you need more than a chat app, it can schedule routines, monitor things in the background, build apps, and run deeper self-improving automations.

It is built to evolve with you. Accepted work, user corrections, repeated routines, and live tool outcomes are reflected into local memory, prompts, routing, and strategy so the assistant gets more aligned with your workflow instead of acting like every session is day one.

- If you keep rewriting replies to be shorter, it learns to stay concise by default
- If a certain tool path keeps succeeding for a task, it becomes more likely to choose that path again
- If you correct how it briefs, routes, or follows up, future runs reflect that correction

Your data stays with you. Your secrets are encrypted. You keep the final say on risky actions.

| | |
|:--|:--|
| **Keeps up with you** | Persistent memory across every conversation, channel, and restart |
| **Briefs you daily** | Calendar, weather, tasks, alerts, and messaging-channel delivery |
| **Acts safely** | Sandboxed tool execution, approvals, guarded actions, and security logs |
| **Follows up** | Background watchers, scheduled tasks, and routines that run unattended |
| **Reaches you** | One assistant reachable from web UI, CLI, Telegram, and WhatsApp |
| **Researches and builds** | Web search, file work, app deployment, API calls, and grounded summaries |
| **Improves over time** | Self-tune and self-evolve adapt memory, prompts, routing, and strategy from corrections and outcomes |
| **Grows with power users** | Apps, integrations, swarm agents, reusable skills, and self-improving autonomy features |

---

## Architecture

```
  Telegram / WhatsApp / Web UI / CLI
          │
          ▼
  ┌───────────────────────────┐
  │       HTTP Gateway        │  ← API, channels, auth, rate limiting
  └─────────────┬─────────────┘
                │
    ┌───────────┼───────────┐
    ▼           ▼           ▼
 ┌──────┐  ┌────────┐  ┌──────────┐
 │Agent │  │Sentinel│  │ Executor │
 │Engine│  │ & Cron │  │ Sandbox  │
 └──┬───┘  └────┬───┘  └────┬─────┘
    │           │            │
    ▼           ▼            ▼
 ┌──────────────────────────────────┐
 │  PostgreSQL + Encrypted Storage  │
 │  Memory · Tasks · Documents ·    │
 │  Secrets · Execution History     │
 └──────────────────────────────────┘
```

**Agent Engine** — LLM routing, multi-provider support, sub-agent orchestration, self-evolve pipeline  
**Sentinel & Cron** — Background watchers, scheduled tasks, learning loops, health monitoring  
**Executor Sandbox** — WASM (Wasmtime) + Docker isolation for code execution, browser automation, app deployment

---

## Install

### Quick start (Docker Compose)

```bash
git clone https://github.com/agentark-ai/AgentArk.git && cd AgentArk
docker compose up -d
```

Open **http://localhost:8990**, pick your LLM provider in Settings, start chatting.

### Convenience installer

```bash
curl -sSL https://raw.githubusercontent.com/agentark-ai/AgentArk/main/scripts/install.sh | bash
```

Review the script before piping to a shell. For the strongest verification story, use Docker Compose or a pinned GHCR image.

### Published container image

| Image | Includes | Size (linux/amd64) |
|:--|:--|:--|
| `ghcr.io/agentark-ai/agentark:latest` | Full runtime with Playwright, cloudflared, tailscale, WhatsApp bridge | ~4.5 GB |

```bash
docker pull ghcr.io/agentark-ai/agentark:latest    # moving tag
docker pull ghcr.io/agentark-ai/agentark:1.2.3     # pinned version
```

For production or first-time installs, prefer a pinned version tag and verify the attestation first. See [VERIFY.md](VERIFY.md).

### CLI mode

Talk to your agent directly from the terminal:

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

| Action | How |
|:--|:--|
| Chat with the agent | Just type your message |
| Run ArkPulse health check | `run arkpulse` or `check system health` |
| Deploy & manage apps | `deploy a weather dashboard` |
| Search the web | `search for latest AI news` |
| Manage tasks & goals | `show my tasks` |
| Toggle full trace mode | `Ctrl+T` |
| Autocomplete slash commands | `Tab` |
| Exit | `Ctrl+D` or `/exit` |

All capabilities available in the Web UI work in CLI mode — same tools, memory, and integrations.

### Build from source

```bash
# Rust 1.75+
export AGENTARK_DATABASE_URL=postgres://agentark:agentark@localhost:5432/agentark
cargo build --release
./target/release/agentark --headless
```

### Verify before install

1. Verify the release checksum against `SHA256SUMS`
2. Verify the Sigstore keyless signature
3. Verify the GitHub provenance attestation
4. Review [VERIFY.md](VERIFY.md) and [SECURITY.md](SECURITY.md)

### Remote access

Built-in remote access is toggleable from Settings. Cloudflare Quick Tunnel is the default public-link mode (no ports to open, traffic encrypted). For private end-to-end encrypted access, choose Tailscale from Settings.

### Management

```bash
docker compose up -d                            # start
docker compose pull && docker compose up -d     # update to latest
docker compose down                             # stop
docker compose down -v                          # stop and full reset
docker compose logs -f agentark-control         # follow logs
```

---

## How It Stacks Up

### Compared with other self-hosted agents

| Compared with | Strongest at | Where AgentArk is different |
|:--|:--|:--|
| **OpenClaw** | Fast local execution and lightweight setup | AgentArk adds daily-assistant continuity, durable memory, guarded actions, and adaptation from use |
| **PicoClaw** | Small-footprint runtime and low-overhead operation | AgentArk adds multi-channel workflows, deeper automation, richer memory, and adaptive improvement |
| **NanoClaw** | Minimal local operation and simplicity | AgentArk adds background routines, stronger follow-up, broader task execution, and compounding personalization |
| **Agent Zero** | Open-ended autonomy and experimentation | AgentArk adds a stronger trust layer, personal-assistant UX, and a more opinionated adaptation control plane |

In short: choose OpenClaw for a lightweight runtime, PicoClaw for the smallest footprint, NanoClaw for simplicity, Agent Zero for open-ended autonomy, or **AgentArk if you want one self-hosted agent that can chat, automate, remember, follow up, and improve over time**.

### Performance snapshot

|                    | AgentArk | NanoBot | PicoClaw | ZeroClaw |
|:-------------------|:---------|:--------|:---------|:---------|
| **Cold start**     | **48ms** | 800ms   | 90ms     | 60ms     |
| **Idle RAM**       | **34MB** | 85MB    | 18MB     | 12MB     |
| **Binary**         | 38MB     | 45MB    | 12MB     | 8MB      |
| **Language**       | Rust     | Python  | Go       | Rust     |
| **Memory system**  | 3-tier   | Basic   | None     | None     |
| **WASM sandbox**   | Yes      | No      | No       | No       |
| **Self-evolution** | Yes      | No      | No       | No       |

> AgentArk is heavier than minimal agents because it bundles a full web UI, WASM sandbox, Playwright browser automation, and a 3-tier memory system. The others are lighter because they do less.

### Monthly cost comparison

All platforms are free/open-source — the real cost is **AI tokens + hosting**.

|                    | NanoClaw      | TinyClaw                | **AgentArk**           |
|:--|:--|:--|:--|
| Software           | Free          | Free (or $30/mo hosted) | Free                   |
| Avg personal use   | $5 - $50/mo   | $10 - $40/mo            | **$2 - $10/mo**        |
| Avg small team     | $20 - $80/mo  | $30 - $70/mo            | **$10 - $30/mo**       |
| Heavy automation   | $50 - $150/mo | $50 - $120/mo           | **$20 - $50/mo**       |
| Risk of bill shock | Medium        | Medium                  | **Low (cheap models)** |

AgentArk's edge: route to DeepSeek, GLM, Mistral, or local Ollama for $0.10-$0.50/1M tokens vs $3-$15/1M on premium models, while still getting verifiable execution, sandboxing, and approval gates.

---

## Features

### Core

| | |
|:--|:--|
| **Sub-Agent Orchestration** | Researcher, Coder, Analyst, Writer, Validator — auto-selected per task |
| **Self-Evolve Engine** | Prompt evolution, policy tuning, strategy learning, and routing benchmarks |
| **Self-Tune** | Learns your style from local history, tracks tool success rates, adjusts autonomy |
| **Cognitive Memory** | Three-tier: Episodic (conversations), Semantic (facts), Procedural (actions) with decay scoring |
| **Live App Deployment** | Deploy static or dynamic apps from chat — Node, Python, HTML, and more |
| **Goal Autopilot** | Goal → plan → scheduled execution → recurring progress reports |
| **Predictive Nudges** | Early warnings for missed deadlines, overdue pressure, recommended next actions |
| **Background Learning** | Periodic reflection, memory consolidation, and pattern induction |

### Security

| | |
|:--|:--|
| **AES-256-GCM + Argon2id** | Secrets encrypted at rest; master-password mode derives keys with Argon2id |
| **Action Security Guard** | Integrity signing, static analysis, permissions, injection scanning |
| **Prompt Protection** | Injection detection, leakage prevention, output redaction |
| **Sandboxed Execution** | WASM (Wasmtime) + Docker isolation with automatic rollback |
| **Execution Proofs** | Verifiable records of what the agent actually did |
| **10-Layer Hardening** | API key auth, localhost bind, CORS, rate limiting, Docker socket proxy, optional TLS |

### Integrations

| | |
|:--|:--|
| **Channels** | Telegram, WhatsApp (Baileys + Cloud API), Web UI |
| **LLM Providers** | Ollama, Anthropic, OpenAI, OpenRouter, any OpenAI-compatible API |
| **Connectors** | GitHub, Notion, Twitter/X, Google Places, 1Password, Twilio, Shopify |
| **MCP Servers** | HTTP JSON-RPC + stdio transports, hot-reload, encrypted credentials |
| **Media** | Image gen (DALL-E, Stability, Fal, Replicate), Video gen (Runway, Luma), Audio (Whisper) |
| **Utilities** | PDF generation, expense tracking, invoice creation, daily briefing, weekly review |

### Autonomy Control Plane

- **Daily Command Brief** — risks, opportunities, and 3 executable recommendations at login
- **Autopilot Modes** — `Focus`, `Ops`, `Travel`, `Finance` — declarative routines + watchers
- **Smart Inbox Triage** — auto-clusters messages: Act now / Delegate / Ignore
- **Live Incident Copilot** — executable containment/recovery playbooks
- **Cross-Channel Continuity** — configurable `per_channel` or `global` context scope
- **Outcome Timeline + Rollback** — replayable event timeline with safe rollback
- **Trust Layer** — risk scoring, policy-based blocking, approval escalation
- **One-Click Delegation Swarm** — delegate strategic tasks to specialist sub-agents

### External launchers (optional)

AgentArk can manage optional external launchers through the Apps view: **Claude Code**, **Codex**, and **OpenCode**. These are companion tools AgentArk can prepare or invoke — not AgentArk modes or rebrands.

```bash
# Docker-hosted
docker exec -it agentark ollama launch claude
docker exec -it agentark ollama launch codex
docker exec -it agentark ollama launch opencode

# Host-native
ollama launch claude
ollama launch codex
ollama launch opencode
```

---

## Configuration

### First-time setup

**Web UI:**
1. Open **http://localhost:8990**
2. Go to **Settings** → pick your **LLM Provider** → enter credentials
3. Set **Bot Name** and **Personality** → Save → start chatting

**CLI:**
1. Run `agentark setup` → pick your model/provider
2. Run `agentark chat`

### LLM providers

| Provider | Base URL | Example models |
|:--|:--|:--|
| **Ollama** (local) | `http://localhost:11434` | `llama3.2`, `qwen2.5`, `mistral` |
| **OpenRouter** | `https://openrouter.ai/api/v1` | `glm-4`, `qwen/qwen-2.5-72b-instruct` |
| **Anthropic** | built-in | `claude-sonnet-4-20250514` |
| **OpenAI** | built-in | `gpt-4o`, `gpt-4-turbo` |
| **OpenAI-compatible** | your URL | any compatible model |

### Telegram bot (optional)

1. Create a bot via [@BotFather](https://t.me/BotFather)
2. Enable Telegram in Settings, paste the token
3. Add your user ID to Allowed Users (get it from [@userinfobot](https://t.me/userinfobot))
4. Save → Restart Bot

### Environment variables

| Variable | Default | Description |
|:--|:--|:--|
| `AGENTARK_CONFIG` | `/app/config` | Configuration directory |
| `AGENTARK_DATA` | `/app/data` | Data directory |
| `AGENTARK_BIND` | `127.0.0.1:8990` | HTTP bind address |
| `AGENTARK_DEBUG` | `false` | Enable debug logging |
| `AGENTARK_DATABASE_URL` | _(set by Compose)_ | PostgreSQL connection string |
| `RUST_LOG` | `info` | Log level (`debug`, `info`, `warn`, `error`) |

### Default stack notes

- Docker Compose starts Postgres and all internal services automatically
- Local embeddings use `BAAI/bge-small-en-v1.5` by default; external OpenAI-compatible endpoints supported
- Bundled skills live under `/app/skills`; user data under `/app/data`
- Keep Docker volumes attached when updating — `docker compose down -v` is a full reset

---

## Security

AES-256-GCM encryption at rest. Argon2id key derivation in master-password mode. Approval-gated actions. WASM sandboxing. Verifiable execution history.

AgentArk stores API keys, OAuth tokens, and custom secrets in encrypted `settings:*` KV records. Memory content, integration credentials, and secret-backed placeholders all use the same encrypted storage path. Secrets are resolved at execution time and never appear in LLM-visible tool-call arguments or traces.

| Layer | Properties |
|:--|:--|
| **Data at rest** | AES-256-GCM with Argon2id key derivation |
| **Data in transit** | HTTPS/TLS when configured, or behind a reverse proxy |
| **Runtime isolation** | Sandboxing, approvals, guarded actions, execution history |
| **Auditability** | Execution proofs, security logs, approval records |

### Permission disclosures

- The default Docker stack mounts your selected workspace into AgentArk containers
- The executor service mounts `/var/run/docker.sock` (host-equivalent Docker control)
- The published image includes browser automation, tunnel tooling, and Docker CLI access
- Public exposure, remote tunnels, and broad workspace mounts increase risk

Full details: [SECURITY.md](SECURITY.md) and [VERIFY.md](VERIFY.md)

---

## Design Principles

- **Secure first** — encrypted secrets, approvals, sandboxing, and verifiable records
- **Daily by default** — briefs, reminders, follow-up, and messaging delivery are first-class
- **Memory that compounds** — each useful interaction builds on previous preferences and facts
- **Self-evolving** — corrections, tool outcomes, and benchmarks improve local memory, prompts, and routing
- **Chat-first** — talk to it naturally, not through config files or flowcharts
- **Power when needed** — tasks, watchers, apps, integrations, and swarm agents for deeper work
- **Model-agnostic** — OpenAI, Anthropic, Google, Ollama, or any OpenAI-compatible endpoint
- **Self-hosted** — your hardware, your data, your rules

---

## Why Rust?

| | |
|:--|:--|
| **Performance** | Tokio async runtime, `Arc<RwLock<T>>` concurrency — no GIL bottleneck |
| **Security** | `Zeroizing` auto-clears secrets from memory; zero `unsafe` blocks in the codebase |
| **Type Safety** | Enums, traits, and compile-time guarantees catch bugs before production |
| **Single Binary** | One compiled binary + Docker — no dependency hell |
| **WASM Sandboxing** | Wasmtime integration is natural in Rust |

---

## API

Full interactive API docs available at **http://localhost:8990/docs#/** after starting AgentArk.

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

- Always use Docker volumes — `docker compose` handles this automatically
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

Contributions welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for the full guide.

### Setup

```bash
git clone https://github.com/agentark-ai/AgentArk.git && cd AgentArk

# Backend (Rust 1.75+)
cargo build && cargo test

# Frontend (Node 20+)
cd frontend && npm install && npm run dev

# Full stack via Docker
AGENTARK_IMAGE=agentark:dev docker compose -f docker-compose.yml -f docker-compose.dev.yml up -d --build
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
skills/             # Built-in skill definitions
```

### Guidelines

- **PRs over issues** — code speaks louder than feature requests
- **One concern per PR** — keep changes focused and reviewable
- **Tests for new features** — add to `tests/` when adding functionality
- **No secrets in code** — use `SecureConfigManager` for anything sensitive
- **Format before push** — `cargo fmt` and `cd frontend && npx prettier --write src/`

---

## Acknowledgments

AgentArk is built on outstanding open-source projects:

| Project | Used for |
|:--|:--|
| [Rust](https://www.rust-lang.org/) | Core runtime — memory safety, performance, fearless concurrency |
| [Tokio](https://tokio.rs/) | Async runtime powering all concurrent operations |
| [Axum](https://github.com/tokio-rs/axum) | HTTP server and API framework |
| [SeaORM](https://www.sea-ql.org/SeaORM/) | Database ORM over PostgreSQL |
| [React](https://react.dev/) + [MUI](https://mui.com/) | Web UI framework and component library |
| [Playwright](https://playwright.dev/) | Browser automation |
| [Lightpanda](https://github.com/lightpanda-io/browser) | Fast headless browser for content extraction |
| [Cloudflared](https://github.com/cloudflare/cloudflared) | Public-link remote access via Cloudflare Tunnel |
| [Tailscale](https://tailscale.com/) | Private tailnet access with end-to-end encryption |
| [Wasmtime](https://wasmtime.dev/) | WebAssembly sandbox for secure code execution |
| [Teloxide](https://github.com/teloxide/teloxide) | Telegram bot framework |

## License

Licensed under either of:

- [MIT](LICENSE-MIT)
- [Apache-2.0](LICENSE-APACHE)

---

<p align="center">
  Built with Rust 🦀
</p>
