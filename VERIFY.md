# Verify AgentArk Releases

Use this document before you install AgentArk on a machine you care about.

The release pipeline now publishes:

- a versioned Linux release tarball
- `SHA256SUMS`
- Sigstore keyless signatures and certificates for the tarball and checksum file
- GitHub artifact attestations for the release tarball, checksum file, and GHCR image
- a `cargo auditable`-built binary so dependency metadata can be inspected after download

The tarball contains the `agentark` binary plus the built web UI assets and runtime support directories (`frontend/dist`, `assets`, `config`, and `skills`) needed for a normal local run.

## Trust Model

Verification should answer three questions:

1. Did I download the exact bytes published for this release?
2. Were those bytes signed by the AgentArk GitHub Actions release workflow?
3. Can I trace the artifact back to a GitHub-hosted build for this repository?

## Tools

Install the tools you need:

- `cosign`: <https://docs.sigstore.dev/>
- GitHub CLI `gh`: <https://cli.github.com/>
- a SHA-256 tool: `sha256sum`, `shasum -a 256`, or PowerShell `Get-FileHash`

## Verify A GitHub Release Tarball

Download these assets from the GitHub release page for the tag you want:

- `agentark-x86_64-unknown-linux-gnu.tar.gz`
- `agentark-x86_64-unknown-linux-gnu.tar.gz.sig`
- `agentark-x86_64-unknown-linux-gnu.tar.gz.pem`
- `SHA256SUMS`
- `SHA256SUMS.sig`
- `SHA256SUMS.pem`

### 1. Verify the signed checksum file

```bash
cosign verify-blob SHA256SUMS \
  --signature SHA256SUMS.sig \
  --certificate SHA256SUMS.pem \
  --certificate-identity-regexp '^https://github.com/agentark-ai/AgentArk/.github/workflows/release.yml@refs/tags/v.*$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

### 2. Verify the tarball checksum against `SHA256SUMS`

Linux:

```bash
grep ' agentark-x86_64-unknown-linux-gnu.tar.gz$' SHA256SUMS | sha256sum --check -
```

macOS:

```bash
shasum -a 256 agentark-x86_64-unknown-linux-gnu.tar.gz
grep ' agentark-x86_64-unknown-linux-gnu.tar.gz$' SHA256SUMS
```

PowerShell:

```powershell
$expected = (Select-String -Path .\SHA256SUMS -Pattern 'agentark-x86_64-unknown-linux-gnu.tar.gz$').Line.Split(' ')[0]
$actual = (Get-FileHash .\agentark-x86_64-unknown-linux-gnu.tar.gz -Algorithm SHA256).Hash.ToLower()
if ($expected -ne $actual) { throw "SHA256 mismatch" }
```

### 3. Verify the tarball signature directly

```bash
cosign verify-blob agentark-x86_64-unknown-linux-gnu.tar.gz \
  --signature agentark-x86_64-unknown-linux-gnu.tar.gz.sig \
  --certificate agentark-x86_64-unknown-linux-gnu.tar.gz.pem \
  --certificate-identity-regexp '^https://github.com/agentark-ai/AgentArk/.github/workflows/release.yml@refs/tags/v.*$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

### 4. Verify GitHub provenance attestation

```bash
gh attestation verify agentark-x86_64-unknown-linux-gnu.tar.gz --repo agentark-ai/AgentArk
```

That verifies the attestation published by the GitHub Actions workflow for the downloaded artifact.

## Verify The Published Container Image

Pull a pinned image by version or digest, not just `latest`.

```bash
docker pull ghcr.io/agentark-ai/agentark:1.2.3
docker inspect --format='{{index .RepoDigests 0}}' ghcr.io/agentark-ai/agentark:1.2.3
```

Take the resulting `sha256:...` digest and verify its GitHub attestation:

```bash
gh attestation verify oci://ghcr.io/agentark-ai/agentark@sha256:REPLACE_WITH_DIGEST --repo agentark-ai/AgentArk
```

## Inspect Dependency Metadata In The Downloaded Binary

The release tarball binary is built with `cargo auditable`, so you can inspect dependency metadata after download.

After extracting the tarball:

```bash
cargo install cargo-audit --locked
cargo audit bin ./agentark-x86_64-unknown-linux-gnu/agentark
```

This does not prove the application is safe. It helps you verify what dependency set is embedded in the release binary and whether RustSec knows about any vulnerable crates in that set.

## Recommended Decision Rule

Install only if all of these are true:

- checksum matches
- `cosign verify-blob` succeeds
- `gh attestation verify` succeeds
- the artifact identity points back to `agentark-ai/AgentArk/.github/workflows/release.yml`
- you are comfortable with the permission model described in [SECURITY.md](SECURITY.md)

If any verification step fails, stop and do not install the artifact.
