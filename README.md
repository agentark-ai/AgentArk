<p align="center">
  <img src="assets/logo.svg" alt="AgentArk Logo" width="420" height="240">
</p>

<h1 align="center">AgentArk 🚀</h1>

<p align="center">
  <em><strong>T</strong>hink. <strong>A</strong>ct. <strong>R</strong>emember. <strong>S</strong>ecurely.</em>
</p>

<p align="center">
  <strong>Self-improving AI agent. Secure by design, not by choice.</strong><br>
  ⚡ Starts in &lt;50ms · ~34MB RAM · Runs on any hardware
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="License"></a>
  <a href="https://github.com/agentark-ai/AgentArk/stargazers"><img src="https://img.shields.io/github/stars/agentark-ai/AgentArk?style=flat" alt="Stars"></a>
  <a href="https://github.com/agentark-ai/AgentArk/issues"><img src="https://img.shields.io/github/issues/agentark-ai/AgentArk" alt="Issues"></a>
  <a href="https://github.com/agentark-ai/AgentArk/pulls"><img src="https://img.shields.io/github/issues-pr/agentark-ai/AgentArk" alt="PRs"></a>
</p>

<p align="center">
  <a href="#install">Getting Started</a> |
  <a href="#features">Features</a> |
  <a href="#configuration">Configuration</a> |
  <a href="#api">API</a>
</p>

<p align="center">
  <strong>Quick Routes:</strong>
  <a href="#install">Install</a> ·
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
  <code>AES-256-GCM encryption · WASM sandbox · action security guard · prompt injection protection · execution proofs</code>
</p>

---

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

### Remote access — one command

```bash
./scripts/start.sh tunnel   # prints a public HTTPS URL via Cloudflare quick tunnel
```

No ports opened, no signup, traffic encrypted end-to-end. Your API key protects all endpoints.

### Management

```bash
./scripts/start.sh              # start (local only)
./scripts/start.sh tunnel       # start + instant remote access
./scripts/start.sh tunnel setup # permanent custom domain (free Cloudflare account)
./scripts/start.sh stop | restart | logs | update | backup | status
```

`update` here is the manual host-side update flow: pull the latest source and rebuild the containers.

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

## Features

### Core

|                             |                                                                                                   |
| --------------------------- | ------------------------------------------------------------------------------------------------- |
| **Parallel Thinking**       | Multiple reasoning paths processed simultaneously — 25-35 % cost reduction                        |
| **Sub-Agent Orchestration** | Researcher · Coder · Analyst · Writer · Validator — auto-selected per task                        |
| **Self-Evolve Engine**      | Policy evolution, strategy tuning, and routing benchmarks that improve the agent over time        |
| **Cognitive Memory**        | Three-tier: Episodic (conversations) · Semantic (facts) · Procedural (actions) with decay scoring |
| **Live App Deployment**     | Deploy static or dynamic apps from chat — Node, Python, HTML, and more                            |
| **Goal Autopilot**          | Goal → plan → scheduled execution → recurring progress reports                                    |
| **Predictive Nudges**       | Early warnings for missed deadlines, overdue pressure, and recommended next actions               |

### Security

|                           |                                                                                                |
| ------------------------- | ---------------------------------------------------------------------------------------------- |
| **AES-256-GCM + Argon2**  | All secrets encrypted at rest; industry-standard key derivation                                |
| **Action Security Guard** | 4-pillar defense: integrity signing, static analysis, permissions, injection scanning          |
| **Prompt Protection**     | Injection detection, leakage prevention, output redaction                                      |
| **Sandboxed Execution**   | WASM (Wasmtime) + Docker isolation with automatic rollback                                     |
| **Execution Proofs**      | Cryptographic receipts for every agent action                                                  |
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

- **Daily Command Brief** — risks, opportunities, and 3 executable recommendations at login
- **Autopilot Modes** — `Focus` · `Ops` · `Travel` · `Finance` — declarative routines + watchers
- **Smart Inbox Triage** — auto-clusters messages: Act now / Delegate / Ignore
- **Live Incident Copilot** — executable containment/recovery playbooks
- **Cross-Channel Continuity** — configurable `per_channel` or `global` context scope
- **Outcome Timeline + Rollback** — replayable event timeline with safe rollback operations
- **Trust Layer** — risk scoring, policy-based blocking, approval escalation
- **One-Click Delegation Swarm** — delegate strategic tasks to specialist sub-agents

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

## Architecture

```
┌───────────────────────────────────────────────────────┐
│              Channels  (HTTP · Telegram · WhatsApp)    │
│                   Web UI @ localhost:8990              │
├───────────────────────────────────────────────────────┤
│                       Agent Core                      │
│   Parallel Thinking ── Sub-Agents ── Security Guard   │
│   Self-Evolve Engine ── Prompt Policy ── Pipeline     │
├───────────────────────────────────────────────────────┤
│                    Cognitive Memory                    │
│       Episodic  ·  Semantic  ·  Procedural            │
├───────────────────────────────────────────────────────┤
│                    Action Runtime                      │
│     WASM Sandbox  ·  Docker Sandbox  ·  Action Guard  │
├───────────────────────────────────────────────────────┤
│   GitHub · Notion · Twitter · Places · Twilio · MCP   │
├───────────────────────────────────────────────────────┤
│       SQLite  ·  Encrypted Secrets  ·  Exec Proofs    │
└───────────────────────────────────────────────────────┘
```

**Data flow:** Input → Security Guard → Parallel Thinking → Sub-Agent Orchestration → Memory Retrieval → Sandboxed Execution → Output Filtering → Encrypted Persistence

---

## API

All endpoints require `Authorization: Bearer <api_key>` (auto-generated on first run).

### Core

| Method     | Path                 | Description                       |
| ---------- | -------------------- | --------------------------------- |
| `POST`     | `/api/chat`          | Send a message and get a response |
| `GET`      | `/api/status`        | Server status and stats           |
| `GET/POST` | `/api/tasks`         | List / create tasks               |
| `GET`      | `/api/notifications` | List notifications                |
| `GET`      | `/api/trace/history` | Execution trace history           |
| `GET`      | `/api/settings`      | Current settings                  |
| `PUT`      | `/api/settings`      | Update settings                   |

### Autonomy

| Method | Path                             | Description                       |
| ------ | -------------------------------- | --------------------------------- |
| `POST` | `/api/autonomy/goals/loop`       | Create autopilot goal loop        |
| `GET`  | `/api/autonomy/goals/progress`   | Goal progress report              |
| `POST` | `/api/autonomy/goals/report_now` | Trigger immediate progress report |
| `GET`  | `/api/briefing`                  | Daily command brief               |
| `GET`  | `/api/nudges`                    | Predictive nudges                 |

### Analytics & Apps

| Method | Path                    | Description                                    |
| ------ | ----------------------- | ---------------------------------------------- |
| `GET`  | `/api/analytics/llm`    | LLM usage analytics (tokens, cost, breakdowns) |
| `GET`  | `/api/apps`             | List deployed apps                             |
| `POST` | `/api/apps/:id/restart` | Restart an app                                 |
| `GET`  | `/apps/:id/`            | Access a deployed app (public, key-gated)      |

### Example: chat

```bash
curl -X POST http://localhost:8990/api/chat \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"message": "What can you do?"}'
```

---

## Why Rust?

|                     |                                                                                   |
| ------------------- | --------------------------------------------------------------------------------- |
| **Performance**     | Tokio async runtime, `Arc<RwLock<T>>` concurrency — no GIL bottleneck             |
| **Security**        | `Zeroizing` auto-clears secrets from memory; zero `unsafe` blocks in the codebase |
| **Type Safety**     | Enums, traits, and compile-time guarantees catch bugs before production           |
| **Single Binary**   | One compiled binary + Docker — no dependency hell                                 |
| **WASM Sandboxing** | Wasmtime integration is natural in Rust; awkward in interpreted languages         |

### Benchmark Snapshot

Local machine quick benchmark (macOS arm64, Feb 2026) — normalized for 0.8 GHz edge hardware.

|                       | **OpenClaw**  | **NanoBot**    | **PicoClaw** | **ZeroClaw** | **AgentArk** |
| --------------------- | ------------- | -------------- | ------------ | ------------ | ------------ |
| **Language**          | TypeScript    | Python         | Go           | Rust         | Rust         |
| **RAM**               | > 1 GB        | > 100 MB       | < 10 MB      | < 5 MB       | ~34 MB       |
| **Startup (0.8 GHz)** | > 500 s       | > 30 s         | < 1 s        | < 10 ms      | < 50 ms      |
| **Binary Size**       | ~28 MB (dist) | N/A (scripts)  | ~8 MB        | ~8.8 MB      | ~56 MB       |
| **Cost**              | Mac Mini $599 | Linux SBC ~$50 | Linux $10    | Any hardware | Any hardware |

> **Notes:** All results are bare-metal, no container overhead. OpenClaw requires Node.js runtime (~390 MB additional memory overhead), NanoBot requires Python runtime. PicoClaw, ZeroClaw, and AgentArk are static binaries. AgentArk's larger binary includes WASM sandbox (Wasmtime), Playwright browser automation, and a full web UI — features the others don't bundle. RAM figures are runtime memory; build-time requirements are higher.

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

- Always use Docker volumes — `docker compose` and `scripts/start.sh` handle this automatically
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

- **PRs over issues** — code speaks louder than feature requests
- **One concern per PR** — keep changes focused and reviewable
- **Tests for new features** — add to `tests/` when adding functionality
- **No secrets in code** — use `SecureConfigManager` for anything sensitive
- **Format before push** — `cargo fmt` and `cd frontend && npx prettier --write src/`

## License

MIT OR Apache-2.0

---

<p align="center">
  Built with Rust 🦀
</p>
