# Contributing to AgentArk

Thanks for contributing.

AgentArk is a high-privilege application: it can execute code, access files, use networked integrations, store secrets, and orchestrate automation. Contributions should optimize for correctness, auditability, and predictable operator trust, not just feature velocity.

Read these first:

- [README.md](README.md)
- [SECURITY.md](SECURITY.md)
- [VERIFY.md](VERIFY.md)

## Before You Start

- Keep changes scoped. One concern per pull request.
- Prefer fixing the root cause instead of layering special cases.
- Do not commit secrets, tokens, cookies, exported credentials, or local `.env` files.
- Do not open public issues for suspected vulnerabilities. Use the private security reporting path described in [SECURITY.md](SECURITY.md).

## Development Setup

### Docker-first local run

This is the easiest way to run the full stack with the same general shape as the published container image:

```bash
git clone https://github.com/agentark-ai/AgentArk.git
cd AgentArk
docker compose up -d --build
```

Open `http://localhost:8990` after the stack is healthy.

### Source-oriented development

Backend:

```bash
cargo build
cargo test
```

Frontend:

```bash
cd frontend
npm install
npm run dev
```

If your change affects the full app flow, validate it against the Docker Compose stack as well.

## Repository Map

```text
src/
|- core/             Agent engine, orchestration, prompts, routing
|- actions/          Tool implementations
|- channels/         HTTP API, chat and messaging adapters
|- security/         Action guards, safety rules, outbound controls
|- runtime/          Execution runtime and sandbox logic
|- storage/          Persistence, migrations, entities
|- integrations/     Provider and integration modules
|- extension_packs/  Generic pack registry/runtime

frontend/src/        React + TypeScript web UI
config/              Default config templates
skills/              Built-in skill definitions
tests/               Cross-cutting test coverage
```

## Workflow Expectations

1. Start from a fresh branch.
2. Make the smallest coherent change that solves the problem.
3. Add or update tests when behavior changes.
4. Update docs when install, security, trust, or operator-facing behavior changes.
5. Open a pull request with a clear summary, validation notes, and any known caveats.

## Code Standards

- Prefer straightforward, explainable code over clever indirection.
- Keep interfaces explicit. Hidden behavior is hard to review and harder to trust.
- Do not weaken approval, audit, or secret-handling paths to make a feature easier.
- Keep provider-specific behavior isolated instead of leaking it into shared contracts.
- Avoid introducing new high-privilege capabilities without documenting the operator impact.

## Validation

Run the narrowest useful validation for the area you changed.

Typical Rust checks:

```bash
cargo check
cargo test
```

Frontend:

```bash
cd frontend
npm run build
```

Formatting:

```bash
cargo fmt
cd frontend && npx prettier --write src/
```

Security and supply-chain checks are enforced in GitHub Actions. If you change dependencies, licenses, release packaging, or install flows, expect the security workflows to matter as much as the feature tests.

## Security-Sensitive Changes

Be explicit in the PR description if your change touches any of these:

- shell or process execution
- file read/write paths
- browser automation
- Docker runtime behavior
- integration credentials or OAuth flows
- public ingress, webhooks, or tunnels
- approval bypasses or permission model changes
- release packaging, signing, or provenance

For these changes, include:

- threat or abuse case considered
- operator impact
- rollback plan if behavior is wrong

## Dependency Changes

When adding or changing dependencies:

- prefer well-maintained crates/packages with a clear purpose
- avoid unnecessary git-sourced dependencies
- keep license compatibility in mind
- explain why the dependency is needed in the PR

This repository uses automated dependency review, `cargo audit`, `cargo deny`, Dependabot, and Scorecard. Dependency additions should survive that scrutiny.

## Pull Request Content

A good PR description should include:

- what changed
- why it changed
- how you validated it
- whether docs were updated
- any remaining caveats or follow-up work

## Trust Model For Contributors

Contributor changes affect a repo that now ships signed release artifacts, checksums, and provenance attestations. Do not make casual changes to:

- release workflows
- install scripts
- Docker privilege model
- verification docs
- security reporting paths

If you touch those surfaces, explain the trust impact in plain language.
