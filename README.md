<p align="center">
  <img src="assets/banner.png" alt="AgentArk" width="700">
</p>

<p align="center">
  <em>Not an agent. An Ark for agents: build from prompts and tools, deploy as apps, automations, or watchers, monitor every action, secure every boundary, self-evolve from your usage.</em>
</p>

<p align="center">
  <strong>Your AI. Your data. Your ark.</strong>
</p>

<p align="center">
  <a href="#install"><img src="https://img.shields.io/badge/INSTALL-Docker_Compose-2ea44f?style=for-the-badge" alt="Install"></a>
  <a href="#what-is-agentark"><img src="https://img.shields.io/badge/WEB_UI-localhost:8990-7C3AED?style=for-the-badge" alt="Web UI"></a>
  <a href="#license"><img src="https://img.shields.io/badge/LICENSE-MIT_%2F_Apache--2.0-orange?style=for-the-badge" alt="License"></a>
  <a href="#why-rust"><img src="https://img.shields.io/badge/RUST-294K_lines-B7410E?style=for-the-badge&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="#install"><img src="https://img.shields.io/badge/FRONTEND-86K_TypeScript-3178C6?style=for-the-badge&logo=typescript&logoColor=white" alt="TypeScript"></a>
  <a href="https://deepwiki.com/agentark-ai/AgentArk"><img src="https://img.shields.io/badge/DEEPWIKI-Ask_the_codebase-1F6FEB?style=for-the-badge" alt="DeepWiki"></a>
</p>

<p align="center">
  A self-hosted runtime for the full agent lifecycle.<br>
  Build agents from structured prompts, tools, and integrations. Deploy them as live apps, scheduled automations, conditional watchers, or chat sessions.<br>
  Monitor every step through ArkSentinel with action traces, failure classification, and drift detection. Secure every capability boundary with intent classification, output guards, approval gates, and per-action authorization.<br>
  Self-evolve prompts, classifiers, routing policies, and specialist behavior from your own usage.<br>
  Review your day, week, or month through ArkReflect: a local visual panorama of where chat, ArkOrbit, apps, goals, watchers, memory, background agents, usage, and learned workflows clustered.<br>
  Chat, memory, devices, integrations, and reviewable actions, all in one place, all on your machine, private by default.<br>
  <code>~3.1GB full Docker image &middot; ~0.45GB idle containers measured &middot; AES-256-GCM encrypted &middot; model-agnostic</code>
</p>

<p align="center">
  <a href="#install">Install</a> &middot;
  <a href="#features">Features</a> &middot;
  <a href="#configuration">Configuration</a> &middot;
  <a href="#architecture">Architecture</a> &middot;
  <a href="#security">Security</a> &middot;
  <a href="#api">API</a> &middot;
  <a href="#contributing">Contributing</a> &middot;
  <a href="https://deepwiki.com/agentark-ai/AgentArk">DeepWiki</a>
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

> Install the Linear integration and list my assigned issues.

> Connect my Google Calendar and remind me 10 minutes before every meeting.

> Set up a webhook that posts Stripe payment alerts to my Telegram.
```

It does not stop at a reply. It can **save the preference**, **schedule the follow-up**, **deliver the brief**, **draft the reply**, **watch for updates**, **connect an integration**, or **promote the work into a durable task** and come back later.

---

## What Is AgentArk?

**AgentArk is not an agent. It is an Ark for agents.** The Ark is the security layer: the wrapper that contains, observes, and enforces what every agent inside it is allowed to do, and the audit surface where every action becomes reviewable. Agents are the things that run inside the Ark - chat handlers, deployed apps, scheduled automations, conditional watchers, specialist sub-agents dispatched by the router. The Ark is what makes any of them safe to point at your real data.

Inside that boundary AgentArk also builds the agents you ask for, deploys them as apps with public URLs, automations, or watchers, monitors every step, and self-evolves their prompts and policies from your usage. Chat, memory, tasks, integrations, documents, companion devices, and audit trails live together in one private workspace on your machine. It can keep track of your preferences, deliver a daily brief, follow up across channels, schedule routines, monitor things in the background, build apps, and take action safely when you ask.

It is built to evolve with you. Accepted work, user corrections, repeated routines, and live tool outcomes are reflected into local memory, prompts, routing, and strategy so the OS gets more aligned with your workflow instead of acting like every session is day one.

- If you keep rewriting replies to be shorter, it learns to stay concise by default
- If a certain tool path keeps succeeding for a task, it becomes more likely to choose that path again
- If you correct how it briefs, routes, or follows up, future runs reflect that correction

Your data stays with you. Your secrets are encrypted. You keep the final say on risky actions.

> Note: AgentArk currently runs as one global workspace. Project-specific workspaces and project-scoped UI/API behavior are intentionally deferred to phase 2.

| | |
|:--|:--|
| **Command layer** | Chat, plans, approvals, and direct work requests |
| **Memory layer** | Facts, preferences, user data, provenance, rollback, and checks |
| **Automation layer** | Tasks, watchers, routines, schedules, and follow-ups |
| **Agent layer** | Specialist agents, delegation, swarm work, and routing |
| **App layer** | Generated tools, reusable skills, and managed apps |
| **Integration layer** | Gmail, Calendar, Telegram, WhatsApp, Slack, webhooks, APIs, MCP servers, and custom packs |
| **Device layer** | Companion device pairing, scoped grants, and high-risk command approvals |
| **Safety layer** | Sandboxing, secrets, policy checks, action review, and trace history |
| **Evolution layer** | ArkMemory, ArkReflect, ArkSentinel, ArkEvolve, and ArkPulse working together |

---

## Architecture

```text
User Interfaces
Web UI | Chat | CLI | Channels | Devices
        |
        v
AgentArk Control Plane
Mission Control | Chat | Approvals | Settings
        |
        v
AI OS Subsystems
ArkMemory | ArkReflect | ArkSentinel | ArkEvolve | ArkPulse
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

### ArkCore Systems

ArkCore is the operating layer inside AgentArk that keeps memory, follow-up work, learning, and health checks connected instead of treating them as separate tools.

| System | What it does |
|:--|:--|
| **ArkMemory** | Reviews current memory, staged changes, provenance, health, and checks. It reconciles session evidence into durable memory with source attribution, review, rollback, and retention-managed audit history. |
| **ArkReflect** | Turns selected days, weeks, or months into a visual retrospective with semantic clusters, narrative summary, source coverage, working-style rhythm, background-agent activity, and examples. It reads cached derived work units by default and refreshes in background so the UI and API do not block on heavy clustering. Its optional Daily Digest is off by default, runs only after quiet windows, and sends nothing when the day had no meaningful activity. |
| **ArkSentinel** | Spots follow-ups, routine work, and unattended issues, then suggests or handles the next step when policy allows it. |
| **ArkEvolve** | Gives plain-language status for what AgentArk learned, what improved, what is still being tested, and what needs review before promotion. |
| **ArkPulse** | Runs system health checks across runtime, config, integrations, security posture, storage, and automation reliability, then surfaces actionable findings. |

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

| Choice | What it does | Best for |
|:--|:--|:--|
| Fast install | Downloads the published AgentArk image and starts Docker Compose | Most users |
| Source build | Clones the AgentArk release checkout and builds the local `agentark:dev` image with Docker Compose | Users who do not want to pull the AgentArk image from GHCR, or who want to inspect/build the release source locally |

Source build does not pull the published AgentArk runtime image from GHCR. It still downloads Docker build base images and package dependencies needed to compile the local image.
The installer records the selected method, so later `agentark start` and `agentark update` keep using the same published-image or source-build path.

Open **http://localhost:8990**, pick your LLM provider in Settings, start chatting.

> **Use the Web UI.** AgentArk is designed to run through the Docker Compose stack and Mission Control at `http://localhost:8990`. Do not use the native `agentark` CLI as your normal entry point; it is an operator/developer escape hatch and may not initialize or expose the full runtime, approvals, integrations, browser/app tooling, and UI-backed settings.

The supported install path uses Docker Compose defaults plus named Docker volumes for runtime state and preserves those volumes across updates. AgentArk does not create or require a root project `.env`. Generated apps may have framework-owned env files inside their own app directories when required, but secret keys stay in AgentArk's managed secret storage or runtime injection path.

### Managed backups

ArkPulse creates framework-managed backups automatically. By default, AgentArk checks for a fresh managed backup every 14 days and only creates one when Sentinel sees the system as idle; if chats, app work, browser sessions, sandbox containers, or heavy background work are active, the backup is deferred and retried later. Backup work runs in background tasks and child processes, not on the main API request path.

Backups are written under `/app/data/backups` as timestamped artifacts:

- `agentark-managed-*.dump` - Postgres logical dump for conversations, messages, tasks, watchers, settings, memory/document indexes, traces, logs, and other DB-backed state.
- `agentark-managed-*.data.tar.gz` - archive of `/app/data`, excluding the backup directory itself.
- `agentark-managed-*.config.tar.gz` - archive of `/app/config` when that config volume is present.

AgentArk creates the backup directory itself. If backup creation fails, ArkPulse raises a critical data-safety finding and notifies the user; users should not be asked to create the backup folder manually.

**Low-memory systems (2-4 GB RAM):** add the low-memory override to reduce Postgres and service footprint:

```bash
docker compose -f docker-compose.yml -f docker-compose.lowmem.yml up -d
```

The bundled Docker runtime includes Lightpanda for fast free-content fetching and the ArkEvolve GEPA optimizer runtime with DSPy. GEPA uses the same active model configured in AgentArk's Models settings; there is no separate GEPA key, model, button, or `.env` setup. ArkEvolve runs this optimizer automatically only after AgentArk is quiet, enough completed work exists, and the daily cost guardrail allows it. The UI surfaces this as Background improvement status, queue, evidence, and latest result.

For operator inspection, GEPA reads recent evidence from `experience_runs`. Its config, scheduler state, budget ledger, and latest result live in `kv_store` under `gepa_optimizer_config_v1`, `gepa_optimizer_auto_state_v1`, `gepa_optimizer_budget_ledger_v1`, and `gepa_optimizer_last_result_v1`. Queue/run artifacts are file-backed under `/app/.agentark/self_evolve/gepa/{pending,running,completed,failed,runs}`.

### Runtime footprint

These numbers are for the supported Docker Compose install. They were measured from a local `agentark:dev` source build on April 18, 2026; exact values vary by platform, Docker cache state, enabled runtime features, model/provider choice, and active jobs.

| Item | Current expectation |
|:--|:--|
| Full AgentArk Docker image | `agentark:dev` measured at **3.07 GB**. Published full-runtime linux/amd64 images should be in the same range; run `docker image ls agentark:dev` or `docker image ls ghcr.io/agentark-ai/agentark` for the exact local size. |
| Bundled ArkEvolve GEPA runtime | Adds the small `/app/bridges/gepa_optimizer` bridge plus a Python venv with DSPy and model-client dependencies. Expect roughly **120-250 MB** additional uncompressed image size, varying with Python dependency versions. |
| AgentArk process startup | **5-10 ms measured** for the Rust binary command startup inside the rebuilt container. This excludes Docker Compose dependency ordering and Postgres health checks. |
| Full local rebuild | About **12 minutes** on the measured Docker Desktop build with warm dependency caches. The Rust release binary compile dominated the build at **11m 38s**; frontend production build was about **11s**. Clean Docker/Cargo/npm caches can be longer. |
| Docker stack ready after image exists | **47.3 seconds measured** from stopped containers to all services healthy with `docker compose -f docker-compose.yml -f docker-compose.dev.yml up -d --wait` on Docker Desktop using the local `agentark:dev` image. This includes Postgres, workspace, executor, control, dependency ordering, and healthcheck intervals. Clean pulls/builds are not included. |
| Idle container memory after fresh start | About **0.5 GB** across the default stack in a local fresh-start measurement. Browser sessions, generated apps, research, local embeddings, and active automations add to this. |
| Low-memory mode | Use `docker-compose.lowmem.yml` on 2-4 GB systems. It caps Postgres at 512 MiB, control at 512 MiB, embeddings at 512 MiB, executor at 512 MiB, workspace at 256 MiB, and reduces Postgres buffers and DB pool sizes. |

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

| Image | Includes | Size (linux/amd64) |
|:--|:--|:--|
| `ghcr.io/agentark-ai/agentark:latest` | Full runtime with Playwright, cloudflared, tailscale, Lightpanda, Docker CLI, Google Workspace CLI, and WhatsApp bridge | ~3.1 GB for current full-runtime linux/amd64 builds; Docker reports the exact size after pull |

```bash
docker pull ghcr.io/agentark-ai/agentark:latest    # moving tag
docker pull ghcr.io/agentark-ai/agentark:1.2.3     # pinned version
```

For production or first-time installs, prefer a pinned version tag and verify the attestation first. See [VERIFY.md](VERIFY.md).

### CLI mode

The native CLI is not the recommended way to use AgentArk. Use the Web UI for normal operation, onboarding, settings, approvals, integrations, apps, and day-to-day chat.

The CLI exists mainly for development, diagnostics, and operator checks inside a configured runtime:

```bash
agentark --pulse                 # run ArkPulse health check
agentark --chat                  # developer/operator chat path
agentark --setup                 # legacy CLI setup path
```

```
    _                    _      _        _
   / \   __ _  ___ _ __ | |_   / \   _ _| | __
  / _ \ / _` |/ _ \ '_ \| __| / _ \ | '__| |/ /
 / ___ \ (_| |  __/ | | | |_ / ___ \| |  |   <
/_/   \_\__, |\___|_| |_|\__/_/   \_\_|  |_|\_\
        |___/
------------------------------------------------------------
                  AgentArk v0.1.0 | CLI Chat
------------------------------------------------------------

Type your message and press Enter.
Commands: /exit  /new  /help

you ➜ what can you do?
agentark ➜ I can help with...
```

| Action | How |
|:--|:--|
| Run a quick health check | `agentark --pulse` |
| Exercise the operator chat path | `agentark --chat` inside a configured runtime |
| Inspect CLI startup behavior | start the binary from the same environment used by the Docker runtime |
| Exit CLI chat | `Ctrl+D` or `/exit` |

The CLI does not replace Mission Control. Some capabilities depend on the web control plane, browser session handoff, approval UI, app management, and Docker Compose service wiring.

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
export AGENTARK_DATABASE_URL=postgres://agentark:agentark@localhost:5432/agentark
cargo build --release
./target/release/agentark --headless
```

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
```

---

## How It Stacks Up

### Feature comparison

| Capability | AgentArk | OpenClaw | Agent Zero | Hermes Agent |
|:--|:--|:--|:--|:--|
| **Primary scope** | Self-improving personal AI Agent OS for chat, memory, tasks, apps, integrations, devices, and audit, with ArkEvolve turning completed work, corrections, tool outcomes, and benchmarks into reviewed memory, procedures, prompt/classifier/specialist updates, routing/strategy policy, and skill candidates | Any-OS gateway that connects chat apps to AI agents | General-purpose autonomous computer assistant | Self-improving terminal agent with a built-in learning loop that creates skills from experience and builds a user model across sessions |
| **UI / control plane** | Mission Control web UI, approvals, settings, trace, apps, and CLI | Browser Control UI served by the gateway | Web UI with interactive streamed output | Full TUI with multiline editing, slash-command autocomplete, interrupt-and-redirect, and streaming tool output; no web dashboard |
| **Memory** | ArkMemory with episodic, semantic, procedural memory, provenance, review, rollback, and retention | Memory core plus optional dreaming/background consolidation | Vector/FAISS memory, knowledge base, project memory, and memory dashboard | Agent-curated memory with periodic nudges, FTS5 full-text session search with LLM summarization, and Honcho dialectic user modeling |
| **Background work** | ArkSentinel with tasks, watchers, routines, schedules, and follow-up loops | Cron jobs, dreaming schedules, gateway events, nodes | Planning/scheduling and autonomous workflows | Built-in cron scheduler that delivers daily reports, nightly backups, and weekly audits to any connected platform in natural language |
| **Health / operations** | ArkPulse health checks across runtime, config, integrations, security posture, storage, and automation reliability | Gateway health, event log, node status, and cron visibility | Web UI session state, memory dashboard, and container status | `hermes doctor` diagnostic plus `/compress`, `/usage`, and `/insights` slash commands |
| **Learning / adaptation** | ArkEvolve with self-tune, experience consolidation, heuristic reflection, procedural pattern induction, skill impact tracking, routing-policy benchmarks, prompt/classifier/specialist prompt evolution, tests on past examples, limited live rollout, change history, review, and rollback | No reviewed self-editing agent loop documented | No reviewed self-editing agent loop documented | Agent-authored skills that self-improve during use, plus Atropos RL environments and trajectory compression for training future tool-calling models |
| **Messaging / channels** | Web, CLI, Telegram, WhatsApp, Slack, webhooks, MCP, and custom channels | Discord, Google Chat, iMessage, Matrix, Teams, Signal, Slack, Telegram, WhatsApp, Zalo, WebChat, and nodes | Web UI first; external API, MCP, and A2A connectivity | Telegram, Discord, Slack, WhatsApp, Signal, CLI, and Home Assistant through a single gateway, with voice memo transcription |
| **Execution isolation** | WASM sandbox, Docker runtime, approval gates, guarded action review, and execution proofs | Managed browser profile, pairing, node scopes, and exec approvals | Dockerized Linux environment | Six pluggable terminal backends: local, Docker, SSH, Daytona, Singularity, and Modal |
| **Security / trust model** | Cross-layer capability vocabulary, scoped grants, semantic action review, signed action integrity, approval escalation, abuse throttling, output guards, secret redaction, and security events | Gateway auth, browser/device pairing, node scopes, and exec approval controls | Docker isolation plus user-configurable prompts/tools; docs warn it can perform dangerous computer actions | Command approval, DM pairing, and container isolation |
| **Audit / accountability** | Trace history, approval records, security logs, execution proofs, action review snapshots, companion audit chain, and rollback-aware memory provenance | Gateway logs, live event log, cron run history, and routed conversation history | Session logs, memory dashboard, and saved Web UI output | FTS5 session search and conversation history through the memory layer; dedicated execution trace not documented |
| **Extensibility** | Skills, extension packs, plugins, MCP servers, custom messaging channels, and companion devices | Skills, plugins, MCP server/client, browser profiles, nodes, and channel plugins | Tools, instruments, skills, extensions, MCP, and A2A | 40+ built-in tools, agent-authored self-improving skills, and MCP server integration |
| **Multi-agent** | Specialist agents, delegation, routing, and swarms | Agent sessions and routed harnesses; node-backed capabilities | Subordinate agents and A2A | Isolated subagents spawned for parallel workstreams |
| **Device / app layer** | Generated apps, app runtime, companion-device grants, and scoped approvals | Mobile/headless nodes with pairing and command surfaces | Operates a Docker computer; companion-device grant system not documented | Runs on Linux, macOS, WSL2, and Android via Termux; Home Assistant integration via gateway; managed app runtime not documented |

> **On "self-improving":** AgentArk and Hermes Agent use the phrase for different systems. AgentArk's self-evolve loop is broader and more controlled: completed or corrected runs become memory, lessons, and procedural patterns; ArkEvolve proposes changes to prompts, request classification, specialist prompts, tool/routing strategy, routing policy, and skills; review candidates are gate-checked before approval, while prompt and policy candidates are benchmarked before limited live rollout; changes keep history, impact signals, automatic stops for clear prompt regressions, and rollback or disable paths where supported. Hermes' main docs show a strong learning loop centered on agent-managed skills, bounded persistent memory, session search, and user modeling; they do not show a comparable multi-surface approval-and-rollout system. Same word, different mechanism and scope.

Source notes: [OpenClaw overview](https://docs.openclaw.ai/), [OpenClaw Control UI](https://docs.openclaw.ai/web/control-ui), [OpenClaw browser](https://docs.openclaw.ai/tools/browser), [OpenClaw nodes](https://docs.openclaw.ai/nodes), [OpenClaw memory dreaming](https://docs.openclaw.ai/concepts/memory), [Hermes Agent GitHub](https://github.com/NousResearch/hermes-agent), [Hermes skills](https://hermes-agent.nousresearch.com/docs/user-guide/features/skills), [Hermes memory](https://hermes-agent.nousresearch.com/docs/user-guide/features/memory), [Hermes checkpoints](https://hermes-agent.nousresearch.com/docs/user-guide/checkpoints-and-rollback), [Agent Zero GitHub](https://github.com/agent0ai/agent-zero), [Agent Zero memory](https://www.agent-zero.ai/p/docs/memory/), [Agent Zero architecture](https://www.agent-zero.ai/p/architecture/).

In short: choose OpenClaw when the main job is routing many chat apps into agents, Hermes Agent when you want a terminal-first agent that builds its own skills and memory across sessions, Agent Zero when you want an open-ended autonomous computer, or **AgentArk if you want one self-hosted AI OS that connects chat, memory, apps, integrations, companion devices, review, and long-running automation in one control plane**.

### Footprint snapshot

| Item | AgentArk |
|:--|:--|
| **Process startup** | **5-10 ms measured** for the Rust binary command startup inside the rebuilt container |
| **Docker stack ready** | **47.3 seconds measured** from stopped containers to all services healthy on Docker Desktop with the local image already built |
| **Idle RAM** | About **0.5 GB** after fresh Docker start |
| **Image size** | **3.07 GB measured** for `agentark:dev` |
| **Runtime shape** | Full Docker Compose stack with web UI, Postgres, WASM sandbox, browser tooling, Docker app runtime support, tunnel tooling, companion-device support, and memory systems |

### Footprint comparison

These competitor values are published requirements or vendor/project claims, not same-machine benchmarks. AgentArk values are measured from the local Docker Desktop rebuild described above.

| Project | Startup / setup | RAM or host guidance | Package / runtime shape | Source |
|:--|:--|:--|:--|:--|
| **AgentArk** | **5-10 ms measured** process startup | About **0.5 GB** idle containers after fresh start | **3.07 GB measured** full Docker image | Local Docker Desktop measurement |
| **OpenClaw** | Official docs say install/onboarding is about **5 minutes** | Official docs list Node 24 or Node 22.14+ and macOS/Linux/Windows; public requirement guides commonly list **4-8 GB RAM** host guidance, with Mac mini guides recommending **8 GB minimum / 16 GB recommended** | Node/Gateway install, optional Docker/local model setup | [OpenClaw install docs](https://docs.openclaw.ai/install), [OpenClaw getting started](https://docs.openclaw.ai/start/getting-started), [OpenClaw requirements guide](https://www.openclawcenter.com/requirements), [Mac mini guide](https://www.openclawcenter.com/install/mac-mini) |
| **Agent Zero** | Docker quick start documented; no official RAM/startup benchmark found | Dockerized Linux environment; host usage depends on tools, browser, memory, and selected models | Docker image with Web UI, memory, browser tools, MCP, A2A, and extensible tools | [Agent Zero GitHub](https://github.com/agent0ai/agent-zero), [Agent Zero architecture](https://www.agent-zero.ai/p/architecture/) |
| **Hermes Agent** | Bash installer documented (`curl \| bash`); no official startup benchmark found | README positions it to run on anything "from a $5 VPS to a GPU cluster" with nearly-free idle cost on serverless; no published RAM benchmark | Single install with six pluggable terminal backends (local, Docker, SSH, Daytona, Singularity, Modal) across Linux, macOS, WSL2, and Android/Termux | [Hermes Agent GitHub](https://github.com/NousResearch/hermes-agent) |

### Cost

AgentArk is free and open-source. Your ongoing cost is whatever LLM provider you configure - nothing more. Every project in this category is the same on that axis.

**What's different about AgentArk is that cheap models actually work.**

Most agent frameworks are thin enough that the model has to do the compensating. If you run DeepSeek, GLM, Mistral, Qwen, or local Ollama inside a thin wrapper, you get malformed tool calls, forgotten context, hallucinated arguments, and broken automations - so users end up on flagship models (Claude Opus, GPT flagship, Gemini Ultra) just to get reliable behavior.

AgentArk's framework compensates instead:

- **Self-heal and retry** catches malformed tool calls, schema errors, and failed dispatches before they reach you
- **Model failover** rotates through your configured providers when one rate-limits or errors, without breaking the conversation
- **ArkMemory** retrieves your preferences instead of asking the model to re-derive them every session
- **Capability correlation and approval gates** catch unsafe outputs at the policy layer, not the model layer
- **ArkEvolve** reinforces prompt shapes that succeed, so the same model performs better over time
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

| | |
|:--|:--|
| **Sub-Agent Orchestration** | Researcher, Coder, Analyst, Writer, Validator - auto-selected per task |
| **Self-Evolve Engine** | Prompt evolution, policy tuning, strategy learning, and routing benchmarks |
| **Self-Tune** | Learns your style from local history, tracks tool success rates, adjusts autonomy |
| **ArkMemory** | Current memory, provenance, review, rollback, and three-tier memory across episodic conversations, semantic facts, and procedural actions |
| **ArkReflect** | Local day/week/month panorama showing where work clustered across chat, ArkOrbit, apps, goals, watchers, memory, Sentinel, ArkPulse, ArkEvolve, usage, and learned workflows |
| **Live App Deployment** | Deploy static or dynamic apps from chat - Node, Python, HTML, and more |
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

**Web UI:**
1. Open **http://localhost:8990**
2. Go to **Settings** → pick your **LLM Provider** → enter credentials
3. Set **Bot Name** and **Personality** → Save → start chatting

**CLI:**
1. Run `agentark --setup` → pick your model/provider
2. Run `agentark --chat`

### LLM providers

| Provider | Base URL | Example models |
|:--|:--|:--|
| **Ollama** (local) | `http://localhost:11434` | `llama3.2`, `qwen2.5`, `mistral` |
| **OpenRouter** | `https://openrouter.ai/api/v1` | `glm-4`, `qwen/qwen-2.5-72b-instruct` |
| **Anthropic** | built-in | `claude-sonnet-4-20250514` |
| **OpenAI** | built-in | `gpt-4o`, `gpt-4-turbo` |
| **Hugging Face Inference** | `https://api-inference.huggingface.co/v1` | `meta-llama/Llama-3.1-8B-Instruct`, `Qwen/Qwen2.5-72B-Instruct` |
| **OpenAI-compatible** | your URL | any compatible model |

### Telegram bot (optional)

1. Create a bot via [@BotFather](https://t.me/BotFather)
2. Enable Telegram in Settings, paste the token
3. Add your user ID to Allowed Users (get it from [@userinfobot](https://t.me/userinfobot))
4. Save → Restart AgentArk

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
- Local embeddings run in an isolated sidecar with `BAAI/bge-small-en-v1.5` by default; external OpenAI-compatible endpoints supported
- User-installed skills live under `/app/data`; app/runtime data stays under Docker volumes
- Keep Docker volumes attached when updating - `docker compose down -v` is a full reset

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

- **Secure first** - encrypted secrets, approvals, sandboxing, and verifiable records
- **Daily by default** - briefs, reminders, follow-up, and messaging delivery are first-class
- **Memory that compounds** - ArkMemory builds on previous preferences, facts, sources, and reviewed memory changes
- **Self-evolving** - corrections, tool outcomes, and benchmarks improve local memory, lessons, procedures, prompts, classifiers, specialist prompts, routing, strategy, and skills
- **Chat-first** - talk to it naturally, not through config files or flowcharts
- **Power when needed** - tasks, watchers, apps, integrations, and swarm agents for deeper work
- **Model-agnostic** - OpenAI, Anthropic, Google, Ollama, or any OpenAI-compatible endpoint
- **Self-hosted** - your hardware, your data, your rules

---

## Why Rust?

| | |
|:--|:--|
| **Performance** | Tokio async runtime, `Arc<RwLock<T>>` concurrency - no GIL bottleneck |
| **Security** | `Zeroizing` auto-clears secrets from memory; zero `unsafe` blocks in the codebase |
| **Type Safety** | Enums, traits, and compile-time guarantees catch bugs before production |
| **Single Binary** | One compiled binary + Docker - no dependency hell |
| **WASM Sandboxing** | Wasmtime integration is natural in Rust |

---

## API

Full interactive API docs available at **http://localhost:8990/docs#/** after starting AgentArk.

### ArkReflect queries

ArkReflect is cached-read by default. Normal reads should use `GET /reflect`; heavy source scans, embedding, and refresh work are queued separately so the web UI and backend do not hang while a retrospective is prepared.

```bash
# Read the cached weekly reflection for an explicit UTC range.
curl "http://localhost:8990/reflect?period=weekly&from=2026-05-01T00:00:00Z&to=2026-05-08T00:00:00Z"

# Queue a guarded background refresh for that same range.
curl -X POST "http://localhost:8990/reflect/refresh?period=weekly&from=2026-05-01T00:00:00Z&to=2026-05-08T00:00:00Z"

# Shortcut: read cached data and request a refresh in the same call.
curl "http://localhost:8990/reflect?period=monthly&from=2026-05-01T00:00:00Z&to=2026-06-01T00:00:00Z&refresh=1"
```

Supported `period` values are `daily`, `weekly`, and `monthly`. `from` and `to` are RFC3339 timestamps; omit them to use the default window for the selected period. Responses include `clusters`, `source_counts`, `baseline_source_counts`, `embedding_status`, `refresh_status`, `cache_status`, `related_history`, and `unclustered_units`.

ArkReflect does not store raw per-message chat embeddings. It creates retention-managed `semantic_work_units` from derived summaries and source metadata, embeds those work units, then clusters and compares them across time windows.

ArkReflect Daily Digest can be enabled in Settings. When enabled, AgentArk prepares a short LLM-written recap after a quiet end-of-day window, stores it in the notification feed, and attempts the selected notification channel. If the structured activity gate finds nothing meaningful, no notification is sent.

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

- Always use Docker volumes - `docker compose` handles this automatically
- If using `docker run`, add `-v agentark-data:/app/data -v agentark-config:/app/config`
</details>

<details>
<summary>Debug logging</summary>

```bash
AGENTARK_DEBUG=true ./scripts/start.sh              # full debug
RUST_LOG=info,agentark=debug ./scripts/start.sh     # agent internals only
```
</details>

---

## Roadmap

- Support multiple accounts per provider across integrations, channels, and reasoning, with workspace-level account management and selection. Project-specific selection can be revisited in phase 2.
- Let users refine an active chat run with extra instructions while AgentArk is still working.
- Add external database support through a dedicated Databases page with read-only schema inspection and conversational querying. Planned providers: Postgres, Supabase, MySQL, Snowflake, and Databricks SQL.
- ArkOrbit is in development for a future release as a dynamic app and widget deployment surface.
- Optional GPU-accelerated learning for power users. Entirely opt-in and self-hosted - your data never leaves your machine, and any improvement is gated by AgentArk's existing rollout-and-promotion pipeline so your live agent only changes when a candidate clearly beats it.
  - Phase 1 (consumer GPU): lightweight on-device learning that makes routine decisions and evaluation faster and cheaper.
  - Phase 2 (workstation GPU): personalization from your own usage - your local agent gradually adapts to your tasks and preferences.
  - Phase 3 (multi-GPU / cluster): heavier on-device training and inference acceleration for teams running serious hardware.

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
AGENTARK_IMAGE=agentark:dev ./scripts/start.sh build
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

### Documentation Map

For documentation generators such as DeepWiki, these are the main product concepts and source areas to index first.

| Area | Start here | Notes |
|:--|:--|:--|
| Product shell | `frontend/src/App.tsx`, `frontend/src/components/NativeWorkspace.tsx`, `frontend/src/styles.css` | Navigation, responsive shell, Mission Control, Chat, ArkMemory, ArkReflect, ArkSentinel, ArkEvolve, ArkPulse, and settings surfaces |
| API surface | `src/channels/http.rs`, `src/channels/http/*` | HTTP routes, settings, integrations, companion devices, model control, webhooks, ArkReflect, ArkPulse, ArkSentinel, and ArkMemory panels |
| Agent runtime | `src/core/agent.rs`, `src/core/agent/*`, `src/runtime/mod.rs` | Tool planning, execution loop, approvals, sandboxing, task routing, generated apps, action traces, and response delivery |
| Memory and learning | `src/core/learning.rs`, `src/core/memory_dedup.rs`, `src/storage/entities/experience_item.rs` | User facts, preferences, ArkMemory views, semantic deduplication, provenance, review, rollback, and consolidation |
| ArkReflect | `src/channels/http/reflect_control.rs`, `src/storage/entities/semantic_work_unit.rs`, `frontend/src/components/pages/ArkReflectPage.tsx` | Cached local retrospectives, derived semantic work units, day/week/month clustering, source coverage, related-history lookup, and Panorama UI |
| ArkSentinel | `src/sentinel.rs`, `src/channels/http/sentinel_panel.rs`, `src/core/autonomy.rs` | Follow-up scanning, routine detection, health findings, proposals, scheduled work, and automation nudges |
| ArkEvolve | `src/core/self_evolve/*`, `src/core/agent/tool_execution.rs` | Prompt, policy, classifier, and specialist evolution with canaries, replay evaluation, promotion gates, and rollback |
| ArkPulse | `src/sentinel.rs`, `src/core/observability.rs`, `src/core/release_updates.rs` | Runtime health checks, remediation hints, operational findings, update status, and system readiness surfaces |
| Integrations and packs | `src/extension_packs/mod.rs`, `src/channels/http/integrations.rs`, `frontend/src/components/IntegrationsPanel.tsx` | Extension packs, messaging channels, OAuth/setup wizards, custom APIs, MCP, webhooks, install/delete cleanup, and secrets handling |
| Companion devices | `src/core/companion.rs`, `src/channels/http/companion_control.rs`, `frontend/src/components/CompanionDevicesPanel.tsx` | Pairing, scoped grants, high-risk approvals, audit trail, device commands, and queued actions |
| Storage and secrets | `src/storage/*`, `src/core/config.rs`, `src/core/secrets.rs`, `src/storage/encrypted.rs` | Postgres entities, schema setup, encrypted config, secret storage, retention, cleanup, and audit data |

Key flows worth documenting:

- Chat request -> plan/tool loop -> trace -> response -> memory and automation updates.
- Memory capture -> semantic deduplication -> review and provenance -> rollback when needed.
- Background session, task, or watcher -> ArkSentinel follow-up -> approval or scheduled action.
- ArkReflect refresh -> bounded source scan -> derived semantic work units -> cached clusters and visual recap.
- Integration install or delete -> config, secrets, files, and audit cleanup.
- App generation and deployment -> sandbox/runtime -> private or public access -> ArkPulse health checks.
- ArkEvolve review candidate -> past-example test -> approval or rejection -> apply or leave unchanged.
- ArkEvolve prompt/policy candidate -> benchmark -> limited live rollout or promotion -> stop, disable, or rollback where supported.

### Guidelines

- **PRs over issues** - code speaks louder than feature requests
- **One concern per PR** - keep changes focused and reviewable
- **Tests for new features** - add to `tests/` when adding functionality
- **No secrets in code** - use `SecureConfigManager` for anything sensitive
- **Format before push** - `cargo fmt` and `cd frontend && npx prettier --write src/`

---

## Acknowledgments

AgentArk stands on the shoulders of a large open-source ecosystem. None of this would exist without the maintainers of these projects:

**Core runtime**

| Project | Used for |
|:--|:--|
| [Rust](https://www.rust-lang.org/) | Core language - memory safety, performance, fearless concurrency |
| [Tokio](https://tokio.rs/) | Async runtime powering all concurrent operations |
| [Hyper](https://hyper.rs/) + [Axum](https://github.com/tokio-rs/axum) + [Tower](https://github.com/tower-rs/tower) | HTTP stack, server, router, middleware |
| [Tokio-Tungstenite](https://github.com/snapview/tokio-tungstenite) | WebSocket implementation (channels + companion devices) |
| [Reqwest](https://github.com/seanmonstar/reqwest) | HTTP client for every outbound API call |
| [Serde](https://serde.rs/) + [serde_json](https://github.com/serde-rs/json) + [TOML](https://github.com/toml-rs/toml) | Serialization across the entire codebase |
| [Tracing](https://github.com/tokio-rs/tracing) | Structured logging and diagnostics |
| [Clap](https://github.com/clap-rs/clap) | CLI argument parsing |
| [Anyhow](https://github.com/dtolnay/anyhow) + [Thiserror](https://github.com/dtolnay/thiserror) | Error handling |

**Storage, data, and scheduling**

| Project | Used for |
|:--|:--|
| [PostgreSQL](https://www.postgresql.org/) + [pgvector](https://github.com/pgvector/pgvector) | Primary database and vector embedding store |
| [SeaORM](https://www.sea-ql.org/SeaORM/) + [SQLx](https://github.com/launchbadge/sqlx) | Database ORM and query layer |
| [Chrono](https://github.com/chronotope/chrono) + [chrono-tz](https://github.com/chronotope/chrono-tz) | Time and timezone handling |
| [cron](https://github.com/zslayton/cron) + [tokio-cron-scheduler](https://github.com/mvniekerk/tokio-cron-scheduler) | Task scheduling and watchers |
| [FastEmbed](https://github.com/Anush008/fastembed-rs) | Local embedding generation for memory and search |
| [pdf-extract](https://github.com/jrmuizel/pdf-extract) | PDF text extraction for documents |

**Security and cryptography**

| Project | Used for |
|:--|:--|
| [Ring](https://github.com/briansmith/ring) + [Rustls](https://github.com/rustls/rustls) | TLS and general-purpose cryptography |
| [ed25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) + [x25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) | Device pairing and signed identities |
| [AES-GCM](https://github.com/RustCrypto/AEADs) + [Argon2](https://github.com/RustCrypto/password-hashes) | Secret-at-rest encryption and password hashing |
| [SHA-2](https://github.com/RustCrypto/hashes) + [BLAKE3](https://github.com/BLAKE3-team/BLAKE3) | Hashing for integrity and content addressing |
| [Zeroize](https://github.com/iqlusioninc/crates/tree/main/zeroize) | Wiping secrets from memory |

**Execution, automation, and integrations**

| Project | Used for |
|:--|:--|
| [Wasmtime](https://wasmtime.dev/) | WebAssembly sandbox for safe code execution |
| [Docker](https://www.docker.com/) + [Bollard](https://github.com/fussybeaver/bollard) | Container runtime and Rust Docker client |
| [Playwright](https://playwright.dev/) | Interactive browser automation and operator handoff |
| [Lightpanda](https://github.com/lightpanda-io/browser) | Fast headless browser for content extraction |
| [russh](https://github.com/warp-tech/russh) | SSH client for remote-action flows |
| [SearXNG](https://github.com/searxng/searxng) | Self-hosted metasearch (optional research backend) |
| [Model Context Protocol](https://modelcontextprotocol.io/) | Open standard for pluggable tool surfaces |
| [Cloudflared](https://github.com/cloudflare/cloudflared) | Public-link remote access via Cloudflare Tunnel |
| [Tailscale](https://tailscale.com/) | Private tailnet access with end-to-end encryption |

**Messaging channels and bots**

| Project | Used for |
|:--|:--|
| [Teloxide](https://github.com/teloxide/teloxide) | Telegram bot framework |
| Community channel SDKs (Slack, Discord, Matrix, WhatsApp, Signal, iMessage, Google Chat, Teams, LINE, WeChat, QQ, and others) | Messaging-channel adapters |

**Frontend and visualization**

| Project | Used for |
|:--|:--|
| [React](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/) | Web UI foundation |
| [Vite](https://vitejs.dev/) | Frontend build and dev server |
| [MUI](https://mui.com/) + [Emotion](https://emotion.sh/) | Component library and CSS-in-JS |
| [TanStack Query](https://tanstack.com/query) | Server-state management and data fetching |
| [Zustand](https://github.com/pmndrs/zustand) | Client-state management |
| [ECharts](https://echarts.apache.org/) + [echarts-for-react](https://github.com/hustcc/echarts-for-react) | Charts and visualizations |
| [react-markdown](https://github.com/remarkjs/react-markdown) + [remark-gfm](https://github.com/remarkjs/remark-gfm) | Markdown rendering in chat |
| [Lucide](https://lucide.dev/) | Icon set |

**Native GUI (optional feature)**

| Project | Used for |
|:--|:--|
| [egui](https://www.egui.rs/) + [eframe](https://github.com/emilk/egui) | Native desktop GUI option |

If we missed a project that's load-bearing for AgentArk, please open a PR - we want to thank everyone.

## License

Licensed under either of:

- [MIT](LICENSE-MIT)
- [Apache-2.0](LICENSE-APACHE)

---

## Beta Notice

AgentArk is still in beta. It can make mistakes, misunderstand requests, choose the wrong tool, produce incorrect output, or take an action that does not match what you intended.

Use it at your own risk, especially around files, secrets, connected accounts, automations, public links, payments, messages, production systems, or anything you cannot easily undo. Keep approvals enabled for risky actions, review plans before applying them, and verify important results yourself.

AgentArk is currently distributed through this Git repository only. `agentark.ai` or any other AgentArk-branded domain is not an official AgentArk property unless it is explicitly linked from this repository.

---

<p align="center">
  Built with Rust 🦀
</p>
