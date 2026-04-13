# Security Policy

AgentArk can execute code, access files, open network connections, store secrets, and integrate with external services. Treat it as high-privilege software and verify it before you install it.

Use [VERIFY.md](VERIFY.md) to validate release assets, checksums, Sigstore signatures, and GitHub provenance attestations before running anything on a host you care about.

## Supported Releases

- Latest tagged GitHub release: supported for security fixes.
- Latest published GHCR image for the most recent release line: supported for security fixes.
- `main`: development branch, best-effort only.
- Older pre-1.0 tags: no long-term support commitment.

## Preferred Install Path

The recommended path is Docker Compose with a dedicated project directory and a dedicated Docker host or VM:

1. Clone the repository and review `docker-compose.yml`, `Dockerfile`, and [VERIFY.md](VERIFY.md).
2. Verify the release artifact or GHCR image before use.
3. Run AgentArk behind localhost or a trusted reverse proxy/tunnel.
4. Set `AGENTARK_MASTER_PASSWORD` so encrypted secrets use Argon2id-derived keys instead of only the local keyfile fallback.

## High-Privilege Disclosures

These are deliberate product capabilities, but they are still trust-sensitive:

- The default Docker stack mounts your chosen workspace directory into AgentArk containers.
- The `agentark-executor` service mounts `/var/run/docker.sock`, which is effectively host-level control over Docker workloads on that machine.
- The full runtime image includes browser automation, tunnel clients, Docker CLI access, and integration helper CLIs.
- AgentArk can hold OAuth tokens, API keys, and other credentials in encrypted local storage.
- If you enable public access, tunnel access, or integrations, you are widening the attack surface beyond a purely local chat UI.

Do not deploy the default stack on a machine where the Docker socket or mounted workspace contains assets you are not willing to expose to a compromised high-privilege application.

## Hardening Guidance

- Prefer Docker Compose or a dedicated VM over running unsigned local debug binaries directly on your host.
- Bind services to `127.0.0.1` unless you intentionally front them with TLS/authenticated ingress.
- Keep the mounted workspace narrow. Do not mount your entire home directory or root filesystem.
- Use separate credentials for integrations, with least-privilege scopes wherever possible.
- Rotate secrets after testing if they were ever exposed in an unsafe environment.
- Keep Docker, the host OS, and browsers up to date.

## Reporting a Vulnerability

Do not open a public issue for a suspected security vulnerability.

Preferred path:

1. Use GitHub Private Vulnerability Reporting / Security Advisories for this repository if it is enabled.
2. If private reporting is unavailable, contact the maintainers through the repository owner's published contact channel and include `[security]` in the subject.

Include:

- affected version, image tag, or commit SHA
- deployment method: Docker, source build, or release artifact
- reproduction steps or proof-of-concept
- impact assessment
- whether credentials or sensitive data were involved

## Security Signals Published By This Repo

- CI runs Rust compile/test checks and frontend builds on pushes and pull requests.
- Security workflows run dependency review on PRs plus `cargo audit` and `cargo deny`.
- Releases publish checksums, Sigstore keyless signatures, and GitHub build provenance attestations.
- GHCR images are published with provenance attestations.
- OpenSSF Scorecard runs on the repository to surface supply-chain hygiene issues.

## Scope Notes

This repository does not claim that every deployment is compliant with regulated environment baselines out of the box. Secure deployment still depends on your host hardening, network exposure, secrets handling, and the scopes you grant to integrated services.
