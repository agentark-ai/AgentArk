# =============================================================================
# AgentArk - AI Agent Docker Image
# =============================================================================
#
# RECOMMENDED: Use docker compose so Postgres, config, and app data come up together
#
#   ./scripts/start.sh  (Linux/Mac)
#   scripts/start.bat   (Windows)
#   docker compose up -d
#
# Your app data (conversations, skills, settings) is automatically preserved
# across updates when using docker compose. The primary database lives in Postgres.
#
# =============================================================================
# DEFAULT IMAGE
# =============================================================================
#
# AgentArk uses a single full-runtime image profile. This is the canonical
# image definition used by docker compose and local container builds.
#
#   docker build -t agentark:dev .
#
# OPTIONAL LIGHTER SELF-BUILDS
# =============================================================================
#
# If you are building only for yourself and want to trim features locally, you
# can still disable individual runtimes with build args:
#
#   docker build -t agentark:dev \
#     --build-arg INSTALL_PLAYWRIGHT_RUNTIME=false \
#     --build-arg INSTALL_TAILSCALE=false \
#     --build-arg INSTALL_CLOUDFLARED=false \
#     --build-arg INSTALL_GWS=false \
#     .
#
# =============================================================================
# MANUAL DOCKER RUN (requires an external Postgres database):
#
# Replace <external-password> with the password for your external database.
#
#   docker run -d -p 8990:8990 \
#     -e AGENTARK_DATABASE_URL=postgres://agentark:<external-password>@host.docker.internal:5432/agentark \
#     -v agentark-data:/app/data \
#     -v agentark-config:/app/config \
#     --name agentark \
#     agentark:latest
#
# WARNING: Running without -v volumes will LOSE YOUR APP DATA on container removal!
# =============================================================================

# =============================================================================
# SECRETS POLICY — READ BEFORE ADDING ENV/ARG LINES
# =============================================================================
# Do NOT set AGENTARK_POSTGRES_PASSWORD, POSTGRES_PASSWORD, AGENTARK_DATABASE_URL,
# or any other secret in this Dockerfile (via ENV, ARG, or COPY). The published
# ghcr.io/agentark-ai/agentark image must be identical for every user and must
# contain zero credentials.
#
# The local Postgres password is generated per-install at `docker compose up`
# time by the pg-bootstrap init service (see docker-compose.yml) into the
# agentark-secrets Docker volume. docker-entrypoint.sh reads it at runtime and
# assembles AGENTARK_DATABASE_URL in memory just before launching the binary.
# =============================================================================

# -- Stage 1: Rust build (with BuildKit cache for fast rebuilds) --
# Use Debian trixie here because fastembed -> ort-sys currently links against
# ONNX Runtime binaries that require glibc 2.38 (__isoc23_* symbols). Bookworm
# ships glibc 2.36, which causes the release link to fail in Docker builds.
FROM rust:1.94-trixie AS builder

ARG TARGETARCH
WORKDIR /app

# Copy manifests for dependency resolution
COPY Cargo.toml Cargo.lock build.rs ./
COPY .cargo ./.cargo

# Create dummy binary targets to cache dependencies. These paths must match
# Cargo.toml because autobin discovery is disabled for predictable test builds.
RUN mkdir -p src/bin && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > src/bin/agentark_embed_server.rs && \
    echo "fn main() {}" > src/bin/prefetch_embeddings.rs

# Default release builds use Cargo.toml's size-optimized profile.
# Use 2 cargo jobs for better build speed without assuming high-memory Docker
# Desktop setups. Pass AGENTARK_BUILD_JOBS=0 to let Cargo choose its default
# parallelism, or a higher number on stronger machines.
ARG AGENTARK_BUILD_JOBS=2

# Cargo profile to build with. Defaults to "release" for published images.
# docker-compose.dev.yml overrides this to "dev-fast" so local dev rebuilds
# skip fat-LTO and reuse incremental fingerprints. Profile name == target
# subdirectory (cargo special-cases "release"; "dev-fast" lives at
# target/dev-fast/...).
ARG AGENTARK_BUILD_PROFILE=release

# Build dependencies with cache mount (survives across docker builds)
RUN --mount=type=cache,id=agentark-cargo-target-${TARGETARCH},target=/app/target \
    --mount=type=cache,id=agentark-cargo-registry-${TARGETARCH},target=/usr/local/cargo/registry \
    if [ "${AGENTARK_BUILD_JOBS}" = "0" ]; then \
        cargo build --locked --profile "${AGENTARK_BUILD_PROFILE}"; \
    else \
        cargo build --locked --profile "${AGENTARK_BUILD_PROFILE}" -j "${AGENTARK_BUILD_JOBS}"; \
    fi && \
    rm -rf src

# Copy source + assets (logo.svg is included at compile time via include_str!)
# CACHEBUST invalidates the layer when source changes aren't detected by Docker
ARG CACHEBUST=0
COPY src ./src
COPY assets ./assets

# Build the selected Cargo profile with cache mount, then copy binaries out of
# cache. We rely on cargo's fingerprint system in target/<profile>/.fingerprint
# to detect source changes via the mtimes preserved by `COPY src ./src` above.
# Do not wipe target/<profile>/agentark or target/<profile>/deps/agentark-*
# before the build: that forces a full re-link on every rebuild and discards
# incremental work that the cache mount is specifically there to preserve.
# After the build, we assert the produced binary is at least as new as any src/
# file so a silently-skipped rebuild can't ship stale code.
RUN --mount=type=cache,id=agentark-cargo-target-${TARGETARCH},target=/app/target \
    --mount=type=cache,id=agentark-cargo-registry-${TARGETARCH},target=/usr/local/cargo/registry \
    if [ "${AGENTARK_BUILD_JOBS}" = "0" ]; then \
        cargo build --locked --profile "${AGENTARK_BUILD_PROFILE}" --bins; \
    else \
        cargo build --locked --profile "${AGENTARK_BUILD_PROFILE}" --bins -j "${AGENTARK_BUILD_JOBS}"; \
    fi && \
    if find src -type f -newer "target/${AGENTARK_BUILD_PROFILE}/agentark" -print -quit | grep -q .; then \
        echo "ERROR: src/ files newer than target/${AGENTARK_BUILD_PROFILE}/agentark after build; aborting" >&2; \
        exit 1; \
    fi && \
    cp "target/${AGENTARK_BUILD_PROFILE}/agentark" /app/agentark-bin && \
    cp "target/${AGENTARK_BUILD_PROFILE}/agentark_embed_server" /app/agentark-embed-server-bin

# Preload the default local embedding model for published/prebuilt images.
# Runtime still falls back to /app/data/embeddings-cache when this cache is not present.
ARG AGENTARK_PREFETCH_LOCAL_EMBEDDINGS=true
RUN --mount=type=cache,id=agentark-cargo-target-${TARGETARCH},target=/app/target \
    --mount=type=cache,id=agentark-cargo-registry-${TARGETARCH},target=/usr/local/cargo/registry \
    --mount=type=cache,id=agentark-local-embeddings-cache,target=/tmp/agentark-embeddings-cache \
    mkdir -p /app/prebuilt-embeddings-cache /tmp/agentark-embeddings-cache && \
    if [ "${AGENTARK_PREFETCH_LOCAL_EMBEDDINGS}" = "true" ]; then \
        "target/${AGENTARK_BUILD_PROFILE}/prefetch_embeddings" /tmp/agentark-embeddings-cache && \
        cp -a /tmp/agentark-embeddings-cache/. /app/prebuilt-embeddings-cache/; \
    else \
        mkdir -p /app/prebuilt-embeddings-cache; \
    fi

# -- Stage 2: Frontend build --
FROM node:22-slim AS frontend-builder
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN --mount=type=cache,id=agentark-frontend-npm,target=/root/.npm \
    npm pkg delete devDependencies.@rollup/rollup-win32-x64-msvc 2>/dev/null; npm ci
ARG FRONTEND_CACHEBUST=0
COPY frontend/src ./src
# public/ carries self-hosted fonts (and any other static passthrough files);
# Vite copies it into dist/ at build time. Without it the served UI 404s on
# /fonts/* and silently falls back to system fonts.
COPY frontend/public ./public
COPY frontend/index.html frontend/tsconfig.json frontend/tsconfig.node.json frontend/vite.config.ts ./
RUN npm run build

# -- Stage 3: Node.js bridges build --
# Build node_modules here (git available), then copy only the result to runtime
FROM node:22-slim AS node-builder

ARG INSTALL_WHATSAPP_BRIDGE=true
ARG INSTALL_PLAYWRIGHT_RUNTIME=false

RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /bridges/whatsapp-bridge
COPY bridges/whatsapp-bridge/package.json bridges/whatsapp-bridge/package-lock.json ./
RUN --mount=type=cache,id=agentark-node-npm,target=/root/.npm \
    if [ "${INSTALL_WHATSAPP_BRIDGE}" = "true" ]; then \
        printf '[url "https://github.com/"]\n\tinsteadOf = ssh://git@github.com/\n\tinsteadOf = git@github.com:\n' > /root/.gitconfig && \
        npm ci --omit=dev && \
        rm -rf /root/.gitconfig /tmp/*; \
    fi
COPY bridges/whatsapp-bridge/index.js ./

# Playwright bridge (skip browser download; runtime image provides browsers)
WORKDIR /bridges/playwright-bridge
ENV PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1
COPY bridges/playwright-bridge/package.json bridges/playwright-bridge/package-lock.json ./
RUN --mount=type=cache,id=agentark-node-npm,target=/root/.npm \
    if [ "${INSTALL_PLAYWRIGHT_RUNTIME}" = "true" ]; then \
        npm ci --omit=dev && \
        rm -rf /tmp/*; \
    fi
COPY bridges/playwright-bridge/index.js ./
COPY bridges/playwright-bridge/manual-login.js ./

# -- Stage 4: Minimal runtime --
# Keep runtime on the same Debian family so the final binary sees the same
# glibc generation it was linked against in the builder stage.
FROM node:22-trixie-slim

ARG INSTALL_PLAYWRIGHT_RUNTIME=false
ARG INSTALL_TAILSCALE=false
ARG INSTALL_CLOUDFLARED=false
ARG INSTALL_LIGHTPANDA=true
ARG INSTALL_GWS=false
ARG INSTALL_DOCKER_CLI=true
ARG INSTALL_OLLAMA_CLI=false

RUN set -eux; \
    apt_packages="ca-certificates curl gosu git postgresql-client python3 python3-pip python3-venv tar binutils build-essential pkg-config"; \
    if [ "${INSTALL_PLAYWRIGHT_RUNTIME}" = "true" ]; then \
        apt_packages="${apt_packages} chromium xvfb x11vnc novnc websockify openbox"; \
    fi; \
    if [ "${INSTALL_DOCKER_CLI}" = "true" ]; then \
        apt_packages="${apt_packages} docker-cli"; \
    fi; \
    if [ "${INSTALL_OLLAMA_CLI}" = "true" ]; then \
        apt_packages="${apt_packages} zstd"; \
    fi; \
    apt-get update; \
    apt-get install -y --no-install-recommends ${apt_packages}; \
    if [ "${INSTALL_PLAYWRIGHT_RUNTIME}" = "true" ] && [ "$(dpkg --print-architecture)" = "amd64" ]; then \
        curl -fsSL https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb -o /tmp/google-chrome-stable.deb; \
        apt-get install -y --no-install-recommends /tmp/google-chrome-stable.deb; \
        rm -f /tmp/google-chrome-stable.deb; \
    fi; \
    rm -rf /var/lib/apt/lists/*

RUN if [ "${INSTALL_TAILSCALE}" = "true" ]; then \
        set -eux; \
        mkdir -p --mode=0755 /usr/share/keyrings; \
        curl -fsSL https://pkgs.tailscale.com/stable/debian/trixie.noarmor.gpg \
            -o /usr/share/keyrings/tailscale-archive-keyring.gpg; \
        curl -fsSL https://pkgs.tailscale.com/stable/debian/trixie.tailscale-keyring.list \
            -o /etc/apt/sources.list.d/tailscale.list; \
        apt-get update; \
        apt-get install -y --no-install-recommends tailscale; \
        rm -rf /var/lib/apt/lists/*; \
    fi

WORKDIR /app

# Create non-root user + all directories in one layer
RUN useradd --create-home --shell /usr/sbin/nologin agent && \
    mkdir -p /app/data /app/data/skills /app/data/whatsapp-auth /app/data/tailscale /app/config /app/bridges/whatsapp-bridge /app/bridges/playwright-bridge && \
    chown -R agent:agent /app

ENV PIP_NO_CACHE_DIR=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PLAYWRIGHT_EXECUTABLE_PATH=/usr/bin/chromium


# Download cloudflared for built-in tunnel support (zero-friction remote access)
# Pinned version for reproducible builds - update deliberately after testing.
ARG CLOUDFLARED_VERSION=2026.2.0
RUN if [ "${INSTALL_CLOUDFLARED}" = "true" ]; then \
        arch="$(dpkg --print-architecture)"; \
        case "${arch}" in \
            amd64|arm64) cloudflared_arch="${arch}" ;; \
            *) echo "ERROR: unsupported cloudflared architecture: ${arch}" >&2; exit 1 ;; \
        esac; \
        curl -fsSL --retry 3 \
            "https://github.com/cloudflare/cloudflared/releases/download/${CLOUDFLARED_VERSION}/cloudflared-linux-${cloudflared_arch}" \
            -o /usr/local/bin/cloudflared && \
        chmod +x /usr/local/bin/cloudflared; \
    fi

# Download Lightpanda for fast headless content extraction (~6MB vs ~1.5GB Chromium)
# Used as fast-path for http_get, web search scraping, and research content fetching.
# Playwright remains for screenshots and complex SPA interaction.
#
# Nightly builds ship with debug symbols (~120 MB on disk). Strip them so the
# layer matches Lightpanda's advertised footprint (~6-20 MB). If stripping ever
# breaks runtime (Zig binaries can have non-standard sections in future
# releases), fall back to `strip --strip-debug` which preserves all symbol
# tables and only removes DWARF info.
ARG LIGHTPANDA_RELEASE=nightly
RUN if [ "${INSTALL_LIGHTPANDA}" = "true" ]; then \
        arch="$(dpkg --print-architecture)"; \
        case "${arch}" in \
            amd64) lightpanda_asset="lightpanda-x86_64-linux" ;; \
            arm64) lightpanda_asset="lightpanda-aarch64-linux" ;; \
            *) echo "ERROR: unsupported Lightpanda architecture: ${arch}" >&2; exit 1 ;; \
        esac; \
        curl -fsSL --retry 3 \
            "https://github.com/lightpanda-io/browser/releases/download/${LIGHTPANDA_RELEASE}/${lightpanda_asset}" \
            -o /usr/local/bin/lightpanda && \
        chmod +x /usr/local/bin/lightpanda && \
        (strip /usr/local/bin/lightpanda || strip --strip-debug /usr/local/bin/lightpanda || true) && \
        /usr/local/bin/lightpanda fetch --dump html "data:text/html,<html><body>ok</body></html>" >/dev/null 2>&1 || \
            { echo "ERROR: stripped lightpanda failed to execute" >&2; exit 1; }; \
    fi

# Install Google Workspace CLI so AgentArk can use gws as a Workspace execution backend.
ARG GOOGLE_WORKSPACE_CLI_VERSION=latest
RUN --mount=type=cache,id=agentark-runtime-npm,target=/root/.npm \
    if [ "${INSTALL_GWS}" = "true" ]; then \
        npm install -g "@googleworkspace/cli@${GOOGLE_WORKSPACE_CLI_VERSION}" && \
        mkdir -p /app/gws-skills && \
        cd /app/gws-skills && \
        (gws generate-skills >/dev/null 2>&1 || true); \
    fi

# Install the Ollama CLI so AgentArk can expose `ollama launch` application registry actions.
ARG OLLAMA_LINUX_URL=https://ollama.com/download/ollama-linux-amd64.tar.zst
RUN if [ "${INSTALL_OLLAMA_CLI}" = "true" ]; then \
        curl -fL \
            --retry 5 \
            --retry-all-errors \
            --retry-delay 5 \
            --connect-timeout 30 \
            --max-time 1800 \
            -o /tmp/ollama-linux-amd64.tar.zst \
            "${OLLAMA_LINUX_URL}" && \
        tar --zstd -xf /tmp/ollama-linux-amd64.tar.zst -C /usr && \
        rm -f /tmp/ollama-linux-amd64.tar.zst; \
    fi

RUN rm -rf /var/lib/apt/lists/*

# Copy pre-built bridges with node_modules (owned by agent).
# The WhatsApp bridge is bundled in the full image and started on demand by AgentArk.
COPY --from=node-builder --chown=agent:agent /bridges/whatsapp-bridge /app/bridges/whatsapp-bridge
COPY --from=node-builder --chown=agent:agent /bridges/playwright-bridge /app/bridges/playwright-bridge


# Copy AgentArk binary from builder
COPY --from=builder --chown=agent:agent /app/agentark-bin /app/agentark
COPY --from=builder --chown=agent:agent /app/agentark-embed-server-bin /app/agentark-embed-server
COPY --from=builder --chown=agent:agent /app/prebuilt-embeddings-cache /app/prebuilt-embeddings-cache

# Copy Python bridge and preinstall DSPy for ArkEvolve GEPA runs.
COPY --chown=agent:agent bridges/gepa_optimizer /app/bridges/gepa_optimizer
RUN python3 -m venv /opt/agentark-gepa && \
    /opt/agentark-gepa/bin/python -m pip install --upgrade pip && \
    /opt/agentark-gepa/bin/python -m pip install -r /app/bridges/gepa_optimizer/requirements.txt && \
    /opt/agentark-gepa/bin/python -c "import dspy" && \
    chown -R agent:agent /opt/agentark-gepa

# Create the runtime config directory. In compose this path is normally
# overlaid by the user-owned agentark-config volume.
RUN mkdir -p /app/config && chown agent:agent /app/config

# Copy assets directly from build context (not part of Rust compilation)
COPY --chown=agent:agent assets /app/assets
# Copy frontend assets (built in Docker, not from host)
COPY --from=frontend-builder --chown=agent:agent /app/frontend/dist /app/frontend/dist
# frontend/legacy is optional (static fallback assets)
RUN mkdir -p /app/frontend/legacy && chown agent:agent /app/frontend/legacy

# Copy entrypoint script (fix Windows CRLF line endings)
COPY --chown=agent:agent docker-entrypoint.sh /app/
RUN sed -i 's/\r$//' /app/docker-entrypoint.sh && chmod +x /app/docker-entrypoint.sh

# Start as root - entrypoint will fix docker socket perms then drop to agent

# Environment
ENV AGENTARK_CONFIG=/app/config
ENV AGENTARK_DATA=/app/data
ENV AGENTARK_LOCAL_EMBEDDINGS_CACHE_DIR=/app/prebuilt-embeddings-cache
ENV TS_STATE_DIR=/app/data/tailscale
ENV TS_SOCKET=/app/data/tailscale/tailscaled.sock
ENV TS_USERSPACE=true
# Playwright automation is optional in the slim image. When enabled, the bridge
# uses a system Chromium binary instead of the full Playwright browser bundle.
ENV PLAYWRIGHT_BROWSERS_PATH=/app/.playwright-browsers
ENV PLAYWRIGHT_HEADLESS=false
ENV PLAYWRIGHT_LIVE_VIEW_PORT=6080
ENV PLAYWRIGHT_LIVE_VIEW_PATH=/vnc.html?autoconnect=1&resize=scale&path=websockify
# Default bridge URL for in-container Playwright service
ENV PLAYWRIGHT_BRIDGE_URL=http://127.0.0.1:3100
ENV PLAYWRIGHT_REAL_BROWSER_NO_SANDBOX=true
# Secure logging: suppress SQLx queries to prevent sensitive data exposure
ENV RUST_LOG=info,sqlx::query=warn,sea_orm=warn,hyper=warn,reqwest=warn

# Expose HTTP API port
EXPOSE 8990
EXPOSE 6080

# Health check
HEALTHCHECK --interval=2s --timeout=2s --start-period=30s --retries=30 \
    CMD python3 -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8990/readiness', timeout=1)" || exit 1

# Run with entrypoint script that checks for volume mounts
ENTRYPOINT ["/app/docker-entrypoint.sh"]
CMD ["--headless"]
