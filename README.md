<p align="center">
  <img src="assets/banner.png" alt="AgentArk" width="700">
</p>

<p align="center">
  <em>Not an agent. An Ark for agents: build from prompts and tools, deploy as apps, automations, or watchers, distill noisy context, monitor every action, secure every boundary, self-evolve from your usage.</em>
</p>

<p align="center">
  <strong>Your AI. Your data. Your ark.</strong>
</p>

<p align="center">
  <a href="#install"><img src="https://img.shields.io/badge/INSTALL-Docker_Compose-2ea44f?style=for-the-badge" alt="Install"></a>
  <a href="#what-is-agentark"><img src="https://img.shields.io/badge/WEB_UI-localhost:8990-7C3AED?style=for-the-badge" alt="Web UI"></a>
  <a href="#license"><img src="https://img.shields.io/badge/LICENSE-MIT_%2F_Apache--2.0-orange?style=for-the-badge" alt="License"></a>
  <a href="ARCHITECTURE.md#why-rust"><img src="https://img.shields.io/badge/RUST-294K_lines-B7410E?style=for-the-badge&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="#install"><img src="https://img.shields.io/badge/FRONTEND-86K_TypeScript-3178C6?style=for-the-badge&logo=typescript&logoColor=white" alt="TypeScript"></a>
  <a href="https://deepwiki.com/agentark-ai/AgentArk"><img src="https://img.shields.io/badge/DEEPWIKI-Ask_the_codebase-1F6FEB?style=for-the-badge" alt="DeepWiki"></a>
</p>

<p align="center">
  A self-hosted runtime for the full agent lifecycle.<br>
  Build agents from structured prompts, tools, and integrations. Deploy them as live apps, scheduled automations, conditional watchers, or chat sessions.<br>
  Monitor every step through Sentinel with action traces, failure classification, and drift detection. Secure every capability boundary with intent classification, output guards, approval gates, and per-action authorization.<br>
  Save context with ArkDistill: deterministic tool-output compaction before noisy browser pages, logs, traces, HTML, and integration dumps reach the model, often cutting noisy outputs by 60-90%.<br>
  Self-evolve prompts, classifiers, routing policies, specialist behavior, and context-saving profiles from your own usage.<br>
  Review your day, week, or month through Reflect: a local visual panorama of where chat, ArkOrbit, apps, goals, watchers, memory, background agents, usage, and learned workflows clustered.<br>
  Chat, memory, devices, integrations, and reviewable actions, all in one place, all on your machine, private by default.<br>
  <code>~3.1GB Docker image &middot; ~500MB idle, ~1GB RAM steady-state under load (5 containers, embeddings loaded) &middot; AES-256-GCM encrypted &middot; model-agnostic</code>
</p>

<p align="center">
  <a href="#install">Install</a> &middot;
  <a href="#features">Features</a> &middot;
  <a href="#ark-core-systems">Ark Core</a> &middot;
  <a href="#configuration">Configuration</a> &middot;
  <a href="#architecture">Architecture</a> &middot;
  <a href="#security">Security</a> &middot;
  <a href="API.md">API</a> &middot;
  <a href="CONTRIBUTING.md">Contributing</a> &middot;
  <a href="https://deepwiki.com/agentark-ai/AgentArk">DeepWiki</a>
</p>

> [!IMPORTANT]
> **AgentArk is in beta — not for production.** It can make mistakes and overwrite files inside its workspace. The Docker boundary keeps it off your host filesystem, but anything you mount into the containers is in scope. Keep approvals on, back up data, verify results.
>
> **Bugs and rough edges are expected.** AgentArk is built and maintained by one person ([@debankadas](https://github.com/debankadas)) in the open, so the surface area is large and fixes ship as they're found. Please [open an issue](https://github.com/agentark-ai/AgentArk/issues) when something breaks — repros, logs, and screenshots help a lot.

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

> Install the Linear integration and list my assigned issues.

> Connect my Google Calendar and remind me 10 minutes before every meeting.

> Set up a webhook that posts Stripe payment alerts to my Telegram.
```

It does not stop at a reply. It can **save the preference**, **schedule the follow-up**, **deliver the brief**, **draft the reply**, **watch for updates**, **connect an integration**, or **promote the work into a durable task** and come back later.

---

## What Is AgentArk?

**AgentArk is not an agent. It is an Ark for agents.** The Ark is the security layer: the wrapper that contains, observes, and enforces what every agent inside it is allowed to do, and the audit surface where every action becomes reviewable. Agents are the things that run inside the Ark - chat handlers, deployed apps, scheduled automations, conditional watchers, specialist sub-agents dispatched by the router. The Ark is what makes any of them safe to point at your real data.

Inside that boundary AgentArk also builds the agents you ask for, deploys them as apps with public URLs, automations, or watchers, monitors every step, distills noisy tool output before it expands the model context, and self-evolves prompts, policies, and context-saving profiles from your usage. Chat, memory, tasks, integrations, documents, companion devices, and audit trails live together in one private workspace on your machine. It can keep track of your preferences, deliver a daily brief, follow up across channels, schedule routines, monitor things in the background, build apps, and take action safely when you ask.

It is built to evolve with you. Accepted work, user corrections, repeated routines, and live tool outcomes are reflected into local memory, prompts, routing, and strategy so the OS gets more aligned with your workflow instead of acting like every session is day one.

- If you keep rewriting replies to be shorter, it learns to stay concise by default
- If a certain tool path keeps succeeding for a task, it becomes more likely to choose that path again
- If browser pages, logs, or traces keep wasting context, Evolve can improve ArkDistill profiles that shrink them while preserving required fields
- If you correct how it briefs, routes, or follows up, future runs reflect that correction

Your data stays with you. Your secrets are encrypted. You keep the final say on risky actions.

> Note: AgentArk currently runs as one global workspace. Project-specific workspaces and project-scoped UI/API behavior are intentionally deferred to phase 2.

|                       |                                                                                           |
| :-------------------- | :---------------------------------------------------------------------------------------- |
| **Command layer**     | Chat, plans, approvals, and direct work requests                                          |
| **Memory layer**      | Facts, preferences, user data, provenance, rollback, and checks                           |
| **Automation layer**  | Tasks, watchers, routines, schedules, and follow-ups                                      |
| **Agent layer**       | Specialist agents, delegation, swarm work, and routing                                    |
| **App layer**         | Generated tools, reusable skills, and managed apps                                        |
| **Integration layer** | Gmail, Calendar, Telegram, WhatsApp, Slack, webhooks, APIs, MCP servers, and custom packs |
| **Device layer**      | Companion device pairing, scoped grants, and high-risk command approvals                  |
| **Safety layer**      | Sandboxing, secrets, policy checks, action review, and trace history                      |
| **Evolution layer**   | Memory, Reflect, Sentinel, Evolve, and Pulse working together              |

---

## Why AgentArk

**Lives where you do.** Docker on your machine, period. Memory, secrets, integration tokens, conversation history, audit trails — all in local volumes, never in someone else's cloud. No managed backend you depend on, no account you have to keep, no telemetry you have to opt out of.

**You pay your model, not us.** Point AgentArk at Ollama or any local model and every prompt after install is genuinely free — no rate limits, no surprise invoice. Bring your own Anthropic, OpenAI, Gemini, or Groq key and you pay the provider's published rate directly; AgentArk never proxies, intermediates, or marks up a single token. No subscription, no per-seat, no minimum.

**Bounded by design.** Every action that touches the world goes through a permission gate. The agent runs inside a Docker boundary with an approval queue for anything not pre-authorized. Your host filesystem stays off-limits unless you explicitly mount what you want it to see.

**Adapts to *you*.** Accepted work, your corrections, and live tool outcomes feed back into local memory, prompts, and routing. Over weeks of use the OS gets shaped by how you actually work — your follow-up style, your routing preferences, the tool paths that keep succeeding for your tasks — not by a generic mix of every other user.

**Open and inspectable.** MIT and Apache 2.0. Read every line, fork it, run it. Audit trails on every action mean you can always see what the agent did, why, and when — across chat, automations, watchers, deployed apps, and integrations.

---

## Install

### Quick start (Docker Compose)

```bash
git clone https://github.com/agentark-ai/AgentArk.git && cd AgentArk
./scripts/start.sh
```

On Windows:

```bat
git clone https://github.com/agentark-ai/AgentArk.git && cd AgentArk
scripts\start.bat
```

Or use the Windows installer:

```powershell
irm https://raw.githubusercontent.com/agentark-ai/AgentArk/main/scripts/install.ps1 | iex
```

The Windows installer checks WSL 2 and Docker Desktop, offers to install Docker Desktop with `winget` when it is missing, starts Docker Desktop when it is installed but stopped, and then asks which AgentArk install method to use:

| Choice       | What it does                                                                                       | Best for                                                                                                            |
| :----------- | :------------------------------------------------------------------------------------------------- | :------------------------------------------------------------------------------------------------------------------ |
| Fast install | Downloads the published AgentArk image and starts Docker Compose                                   | Most users                                                                                                          |
| Source build | Clones the AgentArk release checkout and builds the local `agentark:dev` image with Docker Compose | Users who do not want to pull the AgentArk image from GHCR, or who want to inspect/build the release source locally |

Source build does not pull the published AgentArk runtime image from GHCR. It still downloads Docker build base images and package dependencies needed to compile the local image.
The installer records the selected method, so later `agentark start` and `agentark update` keep using the same published-image or source-build path.

Open **http://localhost:8990**, pick your LLM provider in Settings, start chatting.

> **Use the Web UI.** AgentArk is designed to run through the Docker Compose stack and Mission Control at `http://localhost:8990`.

The supported install path uses Docker Compose defaults plus named Docker volumes for runtime state and preserves those volumes across updates. AgentArk does not create or require a root project `.env`. Generated apps may have framework-owned env files inside their own app directories when required, but secret keys stay in AgentArk's managed secret storage or runtime injection path.

### Managed backups

Pulse creates framework-managed backups automatically. By default, AgentArk checks for a fresh managed backup every 14 days and only creates one when Sentinel sees the system as idle; if chats, app work, browser sessions, sandbox containers, or heavy background work are active, the backup is deferred and retried later. Backup work runs in background tasks and child processes, not on the main API request path.

Backups are written under `/app/data/backups` as timestamped artifacts:

- `agentark-managed-*.dump` - Postgres logical dump for conversations, messages, tasks, watchers, settings, memory/document indexes, traces, logs, and other DB-backed state.
- `agentark-managed-*.data.tar.gz` - archive of `/app/data`, excluding the backup directory itself.
- `agentark-managed-*.config.tar.gz` - archive of `/app/config` when that config volume is present.

AgentArk creates the backup directory itself. If backup creation fails, Pulse raises a critical data-safety finding and notifies the user; users should not be asked to create the backup folder manually.

For full install recovery, also keep an operator volume backup from `./scripts/start.sh backup` or `scripts\start.bat backup`. The automatic managed backup intentionally does not copy the raw `agentark-secrets` volume into `/app/data/backups`; that volume contains install-managed encryption material and should be exported only as part of an intentional, access-controlled backup.

**Low-memory systems (2-4 GB RAM):** add the low-memory override to reduce Postgres and service footprint:

```bash
docker compose -f docker-compose.yml -f docker-compose.lowmem.yml up -d
```

The bundled Docker runtime includes Lightpanda for fast free-content fetching and the Evolve GEPA optimizer runtime with DSPy. GEPA uses the same active model configured in AgentArk's Models settings; there is no separate GEPA key, model, button, or `.env` setup. Evolve runs this optimizer automatically only after AgentArk is quiet, enough completed work exists, and the daily cost guardrail allows it. The UI surfaces this as Background improvement status, queue, evidence, and latest result.

For operator inspection, GEPA reads recent evidence from `experience_runs`. Its config, scheduler state, budget ledger, and latest result live in `kv_store` under `gepa_optimizer_config_v1`, `gepa_optimizer_auto_state_v1`, `gepa_optimizer_budget_ledger_v1`, and `gepa_optimizer_last_result_v1`. Queue/run artifacts are file-backed under `/app/.agentark/self_evolve/gepa/{pending,running,completed,failed,runs}`.

### Runtime footprint

These numbers are for the supported Docker Compose install. They were measured from a local `agentark:dev` source build on April 18, 2026; exact values vary by platform, Docker cache state, enabled runtime features, model/provider choice, and active jobs.

| Item                                    | Current expectation                                                                                                                                                                                                                                                                                                                                             |
| :-------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Full AgentArk Docker image              | `agentark:dev` measured at **3.07 GB**. Published full-runtime linux/amd64 images should be in the same range; run `docker image ls agentark:dev` or `docker image ls ghcr.io/agentark-ai/agentark` for the exact local size.                                                                                                                                   |
| Bundled Evolve GEPA runtime          | Adds the small `/app/bridges/gepa_optimizer` bridge plus a Python venv with DSPy and model-client dependencies. Expect roughly **120-250 MB** additional uncompressed image size, varying with Python dependency versions.                                                                                                                                      |
| AgentArk process startup                | **5-10 ms measured** for the Rust binary command startup inside the rebuilt container. This excludes Docker Compose dependency ordering and Postgres health checks.                                                                                                                                                                                             |
| Full local rebuild                      | About **12 minutes** on the measured Docker Desktop build with warm dependency caches. The Rust release binary compile dominated the build at **11m 38s**; frontend production build was about **11s**. Clean Docker/Cargo/npm caches can be longer.                                                                                                            |
| Docker stack ready after image exists   | **47.3 seconds measured** from stopped containers to all services healthy with `docker compose -f docker-compose.yml -f docker-compose.dev.yml up -d --wait` on Docker Desktop using the local `agentark:dev` image. This includes Postgres, workspace, executor, control, dependency ordering, and healthcheck intervals. Clean pulls/builds are not included. |
| Idle container memory after fresh start | About **0.5 GB** across the default stack in a local fresh-start measurement. Browser sessions, generated apps, research, local embeddings, and active automations add to this.                                                                                                                                                                                 |
| Low-memory mode                         | Use `docker-compose.lowmem.yml` on 2-4 GB systems. It caps Postgres at 512 MiB, control at 512 MiB, embeddings at 512 MiB, executor at 512 MiB, workspace at 256 MiB, and reduces Postgres buffers and DB pool sizes.                                                                                                                                           |

### Optional: run your own SearXNG backend

AgentArk can use a self-hosted SearXNG instance for web search and deep research. AgentArk calls `/search?format=json`, so the SearXNG instance must allow JSON output.

Start one locally in a single command:

```bash
mkdir -p .agentark-searxng && printf 'use_default_settings: true\nserver:\n  secret_key: "agentark-local-searxng"\n  limiter: false\nsearch:\n  formats:\n    - html\n    - json\n' > .agentark-searxng/settings.yml && docker run -d --name agentark-searxng --restart unless-stopped -p 8080:8080 -v "$PWD/.agentark-searxng/settings.yml:/etc/searxng/settings.yml:ro" searxng/searxng:latest
```

Then open AgentArk Settings -> Search and set **SearXNG Base URL (self-hosted)** to:

- `http://host.docker.internal:8080` if AgentArk is running in Docker Desktop
- `http://localhost:8080` if AgentArk is running directly on the host
- `http://<your-host-ip>:8080` or the SearXNG container hostname if AgentArk is running in Docker on Linux

Paste only the base URL and save; AgentArk appends `/search?format=json` itself. No API key is required.

Provider selection logic:

1. If your instance already has an explicit provider order saved in config, AgentArk uses that order for configured providers.
2. Otherwise, AgentArk only considers providers that are actually configured and tries them in this fixed order: Serper -> Brave Search API -> Exa -> Tavily -> Perplexity -> Firecrawl -> SearXNG.
3. If no configured provider is available or they fail, AgentArk falls back to the free chain: DuckDuckGo -> Lightpanda -> Bing RSS.

Paid/API providers are ignored until you save their credentials. Setting a SearXNG base URL is enough to add it to the configured-provider chain. There is currently no up/down control for search provider order in the Settings UI.

### Convenience installer

```bash
curl -sSL https://raw.githubusercontent.com/agentark-ai/AgentArk/main/scripts/install.sh | bash
```

Windows:

```powershell
irm https://raw.githubusercontent.com/agentark-ai/AgentArk/main/scripts/install.ps1 | iex
```

On Windows, choose **Fast install** for the published image path or **Source build** to clone the release source and build `agentark:dev` locally.

Review the script before piping to a shell. For the strongest verification story, use Docker Compose or a pinned GHCR image.

### Published container image

| Image                                 | Includes                                                                                                                | Size (linux/amd64)                                                                            |
| :------------------------------------ | :---------------------------------------------------------------------------------------------------------------------- | :-------------------------------------------------------------------------------------------- |
| `ghcr.io/agentark-ai/agentark:latest` | Full runtime with Playwright, cloudflared, tailscale, Lightpanda, Docker CLI, Google Workspace CLI, and WhatsApp bridge | ~3.1 GB for current full-runtime linux/amd64 builds; Docker reports the exact size after pull |

```bash
docker pull ghcr.io/agentark-ai/agentark:latest    # moving tag
docker pull ghcr.io/agentark-ai/agentark:1.2.3     # pinned version
```

For production or first-time installs, prefer a pinned version tag and verify the attestation first. See [VERIFY.md](VERIFY.md).

### Build from source

Source builds are for contributors and runtime-image development. For normal use, run the Docker Compose stack and use the Web UI.

To build the full Docker runtime from source:

```bash
git clone https://github.com/agentark-ai/AgentArk.git && cd AgentArk
AGENTARK_IMAGE=agentark:dev ./scripts/start.sh build
```

On Windows, the installer can do this for you by choosing **Source build**. From an existing checkout:

```bat
scripts\start.bat build
```

This builds the Docker image locally and starts the same Compose stack as the published-image install. It does not require host Rust or Node installs because dependencies are installed inside the Docker build.

Native host builds are mainly for development and diagnostics:

```bash
# Rust 1.75+
AGENTARK_PG_PASSWORD="$(docker exec agentark-postgres sh -c 'cat /run/secrets/pg_pw')"
export AGENTARK_DATABASE_URL="postgres://agentark:${AGENTARK_PG_PASSWORD}@localhost:${AGENTARK_POSTGRES_PORT:-5432}/agentark"
unset AGENTARK_PG_PASSWORD
cargo build --release
./target/release/agentark --headless
```

The Compose stack generates a per-install Postgres password in the `agentark-secrets` Docker volume and mounts it at `/run/secrets/pg_pw`. If you use your own development Postgres instead of the Compose database, set `AGENTARK_DATABASE_URL` to that database's credentials.

Native source builds do not include the bundled Docker runtime, so AgentArk disables Lightpanda there instead of expecting users to install it separately. Use Docker Compose for the default fully bundled search stack.

### Verify before install

1. Verify the release checksum against `SHA256SUMS`
2. Verify the Sigstore keyless signature
3. Verify the GitHub provenance attestation
4. Review [VERIFY.md](VERIFY.md) and [SECURITY.md](SECURITY.md)

### Remote access

Built-in remote access is toggleable from Settings. Cloudflare Quick Tunnel is the default public-link mode (no ports to open, traffic encrypted). For private end-to-end encrypted access, choose Tailscale from Settings.

### Management

```bash
./scripts/start.sh                              # start
./scripts/start.sh update                       # update to latest
./scripts/start.sh stop                         # stop
docker compose down -v                          # stop and full reset
./scripts/start.sh logs                         # follow logs
./scripts/start.sh backup                       # backup data, config, Postgres, and install secrets
```

---

## Architecture

```text
User Interfaces
Web UI | Chat | Channels | Devices
        |
        v
AgentArk Control Plane
Mission Control | Chat | Approvals | Settings
        |
        v
AI OS Subsystems
Memory | Reflect | Sentinel | Evolve | Pulse
Tasks | Watchers | Apps | Skills | Agents
        |
        v
Execution and Integration Layer
Sandbox | Browser | Webhooks | APIs | Messaging | Companion Devices
        |
        v
Private Storage
Postgres | Documents | Secrets | Traces | Audit Logs
```

**Control Plane** - local web UI, approvals, settings, routing, and runtime supervision
**AI OS Subsystems** - memory, background follow-up, learning, health checks, tasks, watchers, apps, skills, and agents
**Execution Layer** - WASM (Wasmtime) + Docker isolation for code execution, browser automation, app deployment, and integration calls

### Ark Core Systems

Ark Core is the left-nav group for the five core operating surfaces. The product uses short names in the UI and in chat answers: **Memory**, **Reflect**, **Sentinel**, **Evolve**, and **Pulse**.

| System | Plain meaning | Open it when the user wants to... | Default next step | Docs |
| :----- | :------------ | :-------------------------------- | :---------------- | :--- |
| **Memory** | Durable facts, preferences, user data, reusable knowledge, provenance, review, and rollback. | Save or inspect something the agent should remember later. | Review **Memory > Current Memory** and approve, edit, or remove captured items. | [Memory docs](assets/docs/arkmemory.md) |
| **Reflect** | Day, week, and month retrospectives over local work, source coverage, patterns, and background-agent activity. | Understand what happened recently without reading every chat or trace. | Open the weekly view first, then drill into topics or refresh if the cache is stale. | [Reflect docs](assets/docs/arkreflect.md) |
| **Sentinel** | Supervision, follow-ups, routines, observations, approvals, and background learning status. | See what needs attention or what the system is watching in the background. | Check pending proposals and approve, reject, snooze, or let safe routine work continue. | [Sentinel docs](assets/docs/arksentinel.md) |
| **Evolve** | Learning lifecycle for prompts, routing, policies, specialist behavior, tests, canaries, and rollback. | Ask what AgentArk learned, what improved, or what is waiting for review. | Start with the simplified overview, then inspect experiments or review queue items. | [Evolve docs](assets/docs/arkevolve.md) |
| **Pulse** | Runtime health, configuration, integrations, storage, security posture, and safe remediation guidance. | Check whether AgentArk is healthy or why something is failing. | Review the highest-risk finding first, then run or approve safe fixes where available. | [Pulse docs](assets/docs/arkpulse.md) |

ArkOrbit is adjacent to Ark Core: it is the canvas workspace where the agent builds and live-updates widgets, files, and inline tools alongside chat. See [ArkOrbit docs](assets/docs/arkorbit.md).

ArkDistill sits underneath the agent loop: it deterministically shrinks noisy tool output before it reaches model context, often saving 60-90% on noisy browser pages, logs, traces, HTML, and integration dumps. Analytics reports saved tokens, saved cost, and percentage reduction for the selected time range. See [ArkDistill docs](assets/docs/arkdistill.md).

For deeper technical detail, each system has a standalone reference covering its data model, pipeline, HTTP API, UI surface, and known limits.

---

## How It Stacks Up

### Feature comparison

| Capability                 | AgentArk                                                                                                                                                                                                                                                                                                     | OpenClaw                                                                                                   | OpenHuman                                                                                                                                                                                                              | Hermes Agent                                                                                                                                        |
| :------------------------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :--------------------------------------------------------------------------------------------------------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Primary scope**          | Self-improving personal AI Agent OS for chat, memory, tasks, apps, integrations, devices, and audit, with Evolve turning completed work, corrections, tool outcomes, and benchmarks into reviewed memory, procedures, prompt/classifier/specialist updates, and routing/strategy policy | Any-OS gateway that connects chat apps to AI agents                                                        | Local-first desktop AI on Rust + Tauri that ingests 118+ OAuth integrations into a Memory Tree on day one and acts on them; pitched as "memory and doer" across your tools                                              | Self-improving terminal agent with a built-in learning loop that creates skills from experience and builds a user model across sessions             |
| **UI / control plane**     | Mission Control web UI, approvals, settings, trace, apps, and CLI                                                                                                                                                                                                                                            | Browser Control UI served by the gateway                                                                   | Desktop app with clean UI and animated mascot; "no config-first setup, no terminal required"; web dashboard and CLI not documented                                                                                       | Full TUI with multiline editing, slash-command autocomplete, interrupt-and-redirect, and streaming tool output; no web dashboard                    |
| **Memory**                 | Memory with episodic, semantic, procedural memory, provenance, review, rollback, and retention                                                                                                                                                                                                            | Memory core plus optional dreaming/background consolidation                                                | Local-first Memory Tree: integration feeds ingested into canonical ≤3k-token Markdown chunks in local SQLite, browsable and editable through an Obsidian-style wiki                                                      | Agent-curated memory with periodic nudges, FTS5 full-text session search with LLM summarization, and Honcho dialectic user modeling                 |
| **Background work**        | Sentinel with tasks, watchers, routines, schedules, and follow-up loops                                                                                                                                                                                                                                   | Cron jobs, dreaming schedules, gateway events, nodes                                                       | Auto-fetch pulls fresh data from every active connection every 20 minutes and writes into the Memory Tree; general task/watcher/cron surface not documented                                                              | Built-in cron scheduler that delivers daily reports, nightly backups, and weekly audits to any connected platform in natural language               |
| **Health / operations**    | Pulse health checks across runtime, config, integrations, security posture, storage, and automation reliability                                                                                                                                                                                           | Gateway health, event log, node status, and cron visibility                                                | Not documented in public docs                                                                                                                                                                                            | `hermes doctor` diagnostic plus `/compress`, `/usage`, and `/insights` slash commands                                                               |
| **Learning / adaptation**  | Evolve with self-tune, experience consolidation, heuristic reflection, procedural pattern induction, routing-policy benchmarks, prompt/classifier/specialist prompt evolution, deterministic ArkDistill tool-output compaction, context-saving profile optimization, tests on past examples, limited live rollout, change history, review, and rollback; skills remain separately designed or installed capabilities | No reviewed self-editing agent loop documented                                                             | TokenJuice context compression (HTML to Markdown, dedup, URL shortening; about 70% token reduction in their published test); reviewed self-editing agent loop not documented                                            | Agent-authored skills that self-improve during use, plus Atropos RL environments and trajectory compression for training future tool-calling models |
| **Messaging / channels**   | Web, CLI, Telegram, WhatsApp, Slack, webhooks, MCP, and custom channels                                                                                                                                                                                                                                      | Discord, Google Chat, iMessage, Matrix, Teams, Signal, Slack, Telegram, WhatsApp, Zalo, WebChat, and nodes | Native Slack, Gmail, and Notion through the OAuth integration layer plus a live Google Meet agent that joins meetings, transcribes into Memory Tree, and speaks back into the call                                      | Telegram, Discord, Slack, WhatsApp, Signal, CLI, and Home Assistant through a single gateway, with voice memo transcription                         |
| **Execution isolation**    | WASM sandbox, Docker runtime, approval gates, guarded action review, and execution proofs                                                                                                                                                                                                                    | Managed browser profile, pairing, node scopes, and exec approvals                                          | Not documented in public docs                                                                                                                                                                                            | Six pluggable terminal backends: local, Docker, SSH, Daytona, Singularity, and Modal                                                                |
| **Security / trust model** | Cross-layer capability vocabulary, scoped grants, semantic action review, signed action integrity, approval escalation, abuse throttling, output guards, secret redaction, and security events                                                                                                               | Gateway auth, browser/device pairing, node scopes, and exec approval controls                              | Local-first storage and OAuth-scoped integration credentials; explicit approval gates, capability vocabulary, or output guards not documented                                                                            | Command approval, DM pairing, and container isolation                                                                                               |
| **Integration data path**  | **No third party in the integration path.** OAuth tokens AES-256-GCM encrypted in your local Docker volume; integration calls go straight from your machine to the upstream service. MCP servers run locally (stdio) or against endpoints you choose. No managed-cloud proxy, no gateway-style toolkit       | Direct — the gateway runs on your machine, so integration calls leave only to the upstream services you configure | **OAuth tokens and integration API calls transit Composio's infrastructure.** The "118+ via Composio" figure refers to their managed auth + tool-execution toolkit, which sits between OpenHuman and the upstream services it ingests; the resulting data lands locally in the Memory Tree, but the fetch path goes through Composio | Direct — MCP servers and local tools call upstream services from your runtime, no third-party gateway in the integration path                       |
| **Audit / accountability** | Trace history, approval records, security logs, execution proofs, action review snapshots, companion audit chain, and rollback-aware memory provenance                                                                                                                                                       | Gateway logs, live event log, cron run history, and routed conversation history                            | Memory Tree and Obsidian wiki expose ingested data; dedicated execution trace, approval log, or signed-action history not documented                                                                                     | FTS5 session search and conversation history through the memory layer; dedicated execution trace not documented                                     |
| **Extensibility**          | **Open install surface — no vendor catalog limit.** Any MCP server, signed extension pack, plugin, custom messaging channel, or companion device installs through one control plane. Skills and extension packs ship as built-ins; the ceiling is the MCP/pack ecosystem, not a fixed list                                                          | Skills, plugins, MCP server/client, browser profiles, nodes, and channel plugins                           | **118+ one-click OAuth integrations via Composio** plus built-in native tools (web search, scraping, coding utilities, browser/computer control, scheduling, voice); plugin marketplace and MCP not documented                              | 40+ built-in tools, agent-authored self-improving skills, and MCP server integration                                                                |
| **Multi-agent**            | Specialist agents, delegation, routing, and swarms                                                                                                                                                                                                                                                           | Agent sessions and routed harnesses; node-backed capabilities                                              | "Agent coordination" with sub-agent spawning listed in the native toolset                                                                                                                                                | Isolated subagents spawned for parallel workstreams                                                                                                 |
| **Device / app layer**     | Generated apps, app runtime, companion-device grants, and scoped approvals                                                                                                                                                                                                                                   | Mobile/headless nodes with pairing and command surfaces                                                    | Desktop app with native voice (STT/TTS) and browser/computer control; generated apps, public-URL hosting, and companion-device grants not documented                                                                     | Runs on Linux, macOS, WSL2, and Android via Termux; Home Assistant integration via gateway; managed app runtime not documented                      |

> **On "self-improving":** AgentArk and Hermes Agent use the phrase for different systems. AgentArk's self-evolve loop is broader and more controlled: completed or corrected runs become memory, lessons, and procedural patterns; Evolve proposes changes to prompts, request classification, specialist prompts, tool/routing strategy, and routing policy; review candidates are gate-checked before approval, while prompt and policy candidates are benchmarked before limited live rollout; changes keep history, impact signals, automatic stops for clear prompt regressions, and rollback or disable paths where supported. Skills in AgentArk are separately designed or installed capability packages, not the storage layer for learned facts or routine Evolve output. Hermes' main docs show a strong learning loop centered on agent-managed skills, bounded persistent memory, session search, and user modeling; they do not show a comparable multi-surface approval-and-rollout system. Same word, different mechanism and scope.

Source notes: [OpenClaw overview](https://docs.openclaw.ai/), [OpenClaw Control UI](https://docs.openclaw.ai/web/control-ui), [OpenClaw browser](https://docs.openclaw.ai/tools/browser), [OpenClaw nodes](https://docs.openclaw.ai/nodes), [OpenClaw memory dreaming](https://docs.openclaw.ai/concepts/memory), [Hermes Agent GitHub](https://github.com/NousResearch/hermes-agent), [Hermes skills](https://hermes-agent.nousresearch.com/docs/user-guide/features/skills), [Hermes memory](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory), [Hermes checkpoints](https://hermes-agent.nousresearch.com/docs/user-guide/checkpoints-and-rollback), [OpenHuman docs](https://tinyhumans.gitbook.io/openhuman), [OpenHuman GitHub](https://github.com/tinyhumansai/openhuman).

In short: choose OpenClaw when the main job is routing many chat apps into agents, Hermes Agent when you want a terminal-first agent that builds its own skills and memory across sessions, OpenHuman when you want a desktop AI that auto-ingests a fixed catalog of OAuth services into a local Memory Tree on day one (with the trade-off that integration calls and tokens transit Composio's infrastructure), or **AgentArk if you want one self-hosted AI OS that connects chat, memory, apps, integrations, companion devices, review, and long-running automation in one control plane — with an open install surface (any MCP server, signed extension pack, or custom channel) rather than a vendor-curated catalog, and an integration data path that stays between your machine and the upstream service (no third-party gateway, OAuth tokens encrypted locally)**.

### Footprint snapshot

| Item                   | AgentArk                                                                                                                                                                 |
| :--------------------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Process startup**    | **5-10 ms measured** for the Rust binary command startup inside the rebuilt container                                                                                    |
| **Docker stack ready** | **47.3 seconds measured** from stopped containers to all services healthy on Docker Desktop with the local image already built                                           |
| **Idle RAM**           | About **0.5 GB** after fresh Docker start                                                                                                                                |
| **Image size**         | **3.07 GB measured** for `agentark:dev`                                                                                                                                  |
| **Runtime shape**      | Full Docker Compose stack with web UI, Postgres, WASM sandbox, browser tooling, Docker app runtime support, tunnel tooling, companion-device support, and memory systems |

### Footprint comparison

These competitor values are published requirements or vendor/project claims, not same-machine benchmarks. AgentArk values are measured from the local Docker Desktop rebuild described above.

| Project          | Startup / setup                                                                 | RAM or host guidance                                                                                                                                                                                            | Package / runtime shape                                                                                                                             | Source                                                                                                                                                                                                                                                                               |
| :--------------- | :------------------------------------------------------------------------------ | :-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **AgentArk**     | **5-10 ms measured** process startup                                            | About **0.5 GB** idle containers after fresh start                                                                                                                                                              | **3.07 GB measured** full Docker image                                                                                                              | Local Docker Desktop measurement                                                                                                                                                                                                                                                     |
| **OpenClaw**     | Official docs say install/onboarding is about **5 minutes**                     | Official docs list Node 24 or Node 22.14+ and macOS/Linux/Windows; public requirement guides commonly list **4-8 GB RAM** host guidance, with Mac mini guides recommending **8 GB minimum / 16 GB recommended** | Node/Gateway install, optional Docker/local model setup                                                                                             | [OpenClaw install docs](https://docs.openclaw.ai/install), [OpenClaw getting started](https://docs.openclaw.ai/start/getting-started), [OpenClaw requirements guide](https://www.openclawcenter.com/requirements), [Mac mini guide](https://www.openclawcenter.com/install/mac-mini) |
| **OpenHuman**    | Docs position desktop install at about **5 minutes**; no official startup benchmark found                | Not specified in public docs (early beta, public repo created February 2026)                                                                                                                                    | Desktop Tauri app with local SQLite Memory Tree, Obsidian-compatible Markdown wiki, one-click OAuth integration layer for 118+ services, and native voice/browser tools                                       | [OpenHuman docs](https://tinyhumans.gitbook.io/openhuman), [OpenHuman GitHub](https://github.com/tinyhumansai/openhuman)                                                                                                                                                              |
| **Hermes Agent** | Bash installer documented (`curl \| bash`); no official startup benchmark found | README positions it to run on anything "from a $5 VPS to a GPU cluster" with nearly-free idle cost on serverless; no published RAM benchmark                                                                    | Single install with six pluggable terminal backends (local, Docker, SSH, Daytona, Singularity, Modal) across Linux, macOS, WSL2, and Android/Termux | [Hermes Agent GitHub](https://github.com/NousResearch/hermes-agent)                                                                                                                                                                                                                  |

### Cost

AgentArk is free and open-source. Your ongoing cost is whatever LLM provider you configure - nothing more. Every project in this category is the same on that axis.

**What's different about AgentArk is that cheap models actually work.**

Most agent frameworks are thin enough that the model has to do the compensating. If you run DeepSeek, GLM, Mistral, Qwen, or local Ollama inside a thin wrapper, you get malformed tool calls, forgotten context, hallucinated arguments, and broken automations - so users end up on flagship models (Claude Opus, GPT flagship, Gemini Ultra) just to get reliable behavior.

AgentArk's framework compensates instead:

- **Self-heal and retry** catches malformed tool calls, schema errors, and failed dispatches before they reach you
- **Model failover** rotates through your configured providers when one rate-limits or errors, without breaking the conversation
- **Memory** retrieves your preferences instead of asking the model to re-derive them every session
- **ArkDistill** compacts noisy tool output before the next model turn, often saving 60-90% on browser pages, logs, traces, HTML, and integration dumps; already-structured JSON usually saves less
- **Capability correlation and approval gates** catch unsafe outputs at the policy layer, not the model layer
- **Evolve** reinforces prompt shapes and context-saving profiles that succeed, so the same model performs better over time
- **Structured validation** enforces JSON schemas, SSRF checks, and output sanitization independent of the model's reasoning

Net result: a $0.10-$0.50/1M-token model on AgentArk produces reliable results for workloads that would require $3-$15/1M on a thin wrapper.

**Typical ongoing cost:**

- Cheap providers (DeepSeek, GLM, Mistral, OpenRouter free tier): ~$2-$10/month personal, ~$10-$30/month small team
- Local Ollama: $0/month on hardware you already own (Raspberry Pi upward)
- Flagship providers (Claude, GPT, Gemini): $50-$150/month for heavy automation, if you want them

You choose the trade-off at runtime. The core stays the same.

---

## Features

### Core

|                             |                                                                                                                                                                              |
| :-------------------------- | :--------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Sub-Agent Orchestration** | Researcher, Coder, Analyst, Writer, Validator - auto-selected per task                                                                                                       |
| **Self-Evolve Engine**      | Prompt evolution, policy tuning, strategy learning, context-saving ArkDistill profiles, and routing benchmarks                                                                                                   |
| **Self-Tune**               | Learns your style from local history, tracks tool success rates, adjusts autonomy                                                                                            |
| **Memory**               | Current memory, provenance, review, rollback, and three-tier memory across episodic conversations, semantic facts, and procedural actions                                    |
| **Reflect**              | Local day/week/month panorama showing where work clustered across chat, ArkOrbit, apps, goals, watchers, memory, Sentinel, Pulse, Evolve, usage, and learned workflows |
| **Live App Deployment**     | Deploy static or dynamic apps from chat - Node, Python, HTML, and more                                                                                                       |
| **Goal Autopilot**          | Goal → plan → scheduled execution → recurring progress reports                                                                                                               |
| **Predictive Nudges**       | Early warnings for missed deadlines, overdue pressure, recommended next actions                                                                                              |
| **Background Learning**     | Periodic reflection, memory consolidation, and pattern induction                                                                                                             |

### Security

|                            |                                                                                      |
| :------------------------- | :----------------------------------------------------------------------------------- |
| **AES-256-GCM + Argon2id** | Secrets encrypted at rest; master-password mode derives keys with Argon2id           |
| **Action Security Guard**  | Integrity signing, static analysis, permissions, injection scanning                  |
| **Prompt Protection**      | Injection detection, leakage prevention, output redaction                            |
| **Sandboxed Execution**    | WASM (Wasmtime) + Docker isolation with automatic rollback                           |
| **Execution Proofs**       | Verifiable records of what the agent actually did                                    |
| **10-Layer Hardening**     | API key auth, localhost bind, CORS, rate limiting, Docker socket proxy, optional TLS |

### Integrations

|                   |                                                                                          |
| :---------------- | :--------------------------------------------------------------------------------------- |
| **Channels**      | Telegram, WhatsApp (Baileys + Cloud API), Web UI                                         |
| **LLM Providers** | Ollama, Anthropic, OpenAI, OpenRouter, any OpenAI-compatible API                         |
| **Connectors**    | GitHub, Notion, Twitter/X, Google Places, 1Password, Twilio, Shopify                     |
| **MCP Servers**   | HTTP JSON-RPC + stdio transports, hot-reload, encrypted credentials                      |
| **Media**         | Image gen (DALL-E, Stability, Fal, Replicate), Video gen (Runway, Luma), Audio (Whisper) |
| **Utilities**     | PDF generation, expense tracking, invoice creation, daily briefing, weekly review        |

### Autonomy Control Plane

- **Daily Command Brief** - risks, opportunities, and 3 executable recommendations at login
- **Autopilot Modes** - `Focus`, `Ops`, `Travel`, `Finance` - declarative routines + watchers
- **Smart Inbox Triage** - auto-clusters messages: Act now / Delegate / Ignore
- **Live Incident Copilot** - executable containment/recovery playbooks
- **Cross-Channel Continuity** - configurable `per_channel` or `global` context scope
- **Outcome Timeline + Rollback** - replayable event timeline with safe rollback
- **Trust Layer** - risk scoring, policy-based blocking, approval escalation
- **One-Click Delegation Swarm** - delegate strategic tasks to specialist sub-agents

---

## Configuration

### First-time setup

1. Open **http://localhost:8990**
2. Go to **Settings** → pick your **LLM Provider** → enter credentials

### LLM providers

| Provider                   | Base URL                                  | Example models                                                  |
| :------------------------- | :---------------------------------------- | :-------------------------------------------------------------- |
| **Ollama** (local)         | `http://localhost:11434`                  | `llama3.2`, `qwen2.5`, `mistral`                                |
| **OpenRouter**             | `https://openrouter.ai/api/v1`            | `glm-4`, `qwen/qwen-2.5-72b-instruct`                           |
| **Anthropic**              | built-in                                  | `claude-sonnet-4-20250514`                                      |
| **OpenAI**                 | built-in                                  | `gpt-4o`, `gpt-4-turbo`                                         |
| **Hugging Face Inference** | `https://api-inference.huggingface.co/v1` | `meta-llama/Llama-3.1-8B-Instruct`, `Qwen/Qwen2.5-72B-Instruct` |
| **OpenAI-compatible**      | your URL                                  | any compatible model                                            |

### Telegram bot (optional)

1. Create a bot via [@BotFather](https://t.me/BotFather)
2. Enable Telegram in Settings, paste the token
3. Add your user ID to Allowed Users (get it from [@userinfobot](https://t.me/userinfobot))
4. Save → Restart AgentArk

### Environment variables

| Variable                | Default            | Description                                  |
| :---------------------- | :----------------- | :------------------------------------------- |
| `AGENTARK_CONFIG`       | `/app/config`      | Configuration directory                      |
| `AGENTARK_DATA`         | `/app/data`        | Data directory                               |
| `AGENTARK_BIND`         | `127.0.0.1:8990`   | HTTP bind address                            |
| `AGENTARK_DEBUG`        | `false`            | Enable debug logging                         |
| `AGENTARK_DATABASE_URL` | _(set by Compose)_ | PostgreSQL connection string                 |
| `RUST_LOG`              | `info`             | Log level (`debug`, `info`, `warn`, `error`) |

### Default stack notes

- Docker Compose starts Postgres and all internal services automatically
- Local embeddings run in an isolated sidecar with `BAAI/bge-small-en-v1.5` by default; external OpenAI-compatible endpoints supported
- User-installed skills live under `/app/data`; app/runtime data stays under Docker volumes
- Backups must keep `agentark-secrets` together with `agentark-postgres-data` and `agentark-config`; it contains the install-managed encryption secret required to unlock encrypted local data
- Keep Docker volumes attached when updating - `docker compose down -v` is a full reset

---

## Security

AES-256-GCM encryption at rest. Argon2id key derivation in master-password mode. Approval-gated actions. WASM sandboxing. Verifiable execution history.

AgentArk stores API keys, OAuth tokens, and custom secrets in encrypted `settings:*` KV records. Memory content, integration credentials, and secret-backed placeholders all use the same encrypted storage path. Secrets are resolved at execution time and never appear in LLM-visible tool-call arguments or traces.

| Layer                 | Properties                                                |
| :-------------------- | :-------------------------------------------------------- |
| **Data at rest**      | AES-256-GCM with Argon2id key derivation                  |
| **Data in transit**   | HTTPS/TLS when configured, or behind a reverse proxy      |
| **Runtime isolation** | Sandboxing, approvals, guarded actions, execution history |
| **Auditability**      | Execution proofs, security logs, approval records         |

### Permission disclosures

- The default Docker stack mounts your selected workspace into AgentArk containers
- The executor service mounts `/var/run/docker.sock` (host-equivalent Docker control)
- The published image includes browser automation, tunnel tooling, and Docker CLI access
- Public exposure, remote tunnels, and broad workspace mounts increase risk

Full details: [SECURITY.md](SECURITY.md) and [VERIFY.md](VERIFY.md)

---

## More documentation

- **[Design principles & internals](ARCHITECTURE.md)** — design principles, why Rust, project structure, and the documentation map for navigating the codebase
- **[API & Reflect](API.md)** — interactive API reference, Reflect query endpoints, and troubleshooting
- **[Roadmap](ROADMAP.md)** — what's planned and what's intentionally deferred
- **[Contributing](CONTRIBUTING.md)** — development setup, repository map, and contribution workflow
- **[Acknowledgments](ACKNOWLEDGMENTS.md)** — the open-source projects AgentArk is built on

---

## License

Licensed under either of:

- [MIT](LICENSE-MIT)
- [Apache-2.0](LICENSE-APACHE)

---

<p align="center">
  <em>Distributed only through this Git repository — <code>agentark.ai</code> or any other AgentArk-branded domain is not official unless linked from here.</em>
</p>

<p align="center">
  Built with Rust 🦀 by <a href="https://github.com/debankadas">@debankadas</a> — a solo project in beta. Bug reports welcome.
</p>
