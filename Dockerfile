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
# AgentArk now ships as a single full-runtime image. This is the image profile
# used by docker compose and the published GHCR release image.
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
#   docker run -d -p 8990:8990 \
#     -e AGENTARK_DATABASE_URL=postgres://agentark:agentark@host.docker.internal:5432/agentark \
#     -v agentark-data:/app/data \
#     -v agentark-config:/app/config \
#     --name agentark \
#     agentark:latest
#
# WARNING: Running without -v volumes will LOSE YOUR APP DATA on container removal!
# =============================================================================

# -- Stage 1: Rust build (with BuildKit cache for fast rebuilds) --
# Use Debian trixie here because fastembed -> ort-sys currently links against
# ONNX Runtime binaries that require glibc 2.38 (__isoc23_* symbols). Bookworm
# ships glibc 2.36, which causes the release link to fail in Docker builds.
FROM rust:1.94-trixie AS builder

WORKDIR /app

# Copy manifests for dependency resolution
COPY Cargo.toml Cargo.lock build.rs ./
COPY .cargo ./.cargo

# Create dummy main to cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Balanced default: use 2 cargo jobs for better build speed without assuming
# high-memory Docker Desktop setups. Pass AGENTARK_BUILD_JOBS=0 to let Cargo
# choose its default parallelism, or a higher number on stronger machines.
ENV CARGO_PROFILE_RELEASE_LTO=thin
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=4
ARG AGENTARK_BUILD_JOBS=2
ARG AGENTARK_DOCKER_FEATURES="telegram,docker,ssh"

# Build dependencies with cache mount (survives across docker builds)
RUN --mount=type=cache,id=agentark-cargo-target,target=/app/target \
    --mount=type=cache,id=agentark-cargo-registry,target=/usr/local/cargo/registry \
    if [ "${AGENTARK_BUILD_JOBS}" = "0" ]; then \
        cargo build --release --no-default-features --features "${AGENTARK_DOCKER_FEATURES}"; \
    else \
        cargo build --release --no-default-features --features "${AGENTARK_DOCKER_FEATURES}" -j "${AGENTARK_BUILD_JOBS}"; \
    fi && \
    rm -rf src

# Copy source + assets (logo.svg is included at compile time via include_str!)
# CACHEBUST invalidates the layer when source changes aren't detected by Docker
ARG CACHEBUST=0
COPY src ./src
COPY assets ./assets

# Build for release with cache mount, then copy binary out of cache
RUN --mount=type=cache,id=agentark-cargo-target,target=/app/target \
    --mount=type=cache,id=agentark-cargo-registry,target=/usr/local/cargo/registry \
    rm -f target/release/agentark target/release/deps/agentark-* && \
    if [ "${AGENTARK_BUILD_JOBS}" = "0" ]; then \
        cargo build --release --no-default-features --features "${AGENTARK_DOCKER_FEATURES}" --bins; \
    else \
        cargo build --release --no-default-features --features "${AGENTARK_DOCKER_FEATURES}" --bins -j "${AGENTARK_BUILD_JOBS}"; \
    fi && \
    cp target/release/agentark /app/agentark-bin

# Preload the default local embedding model for published/prebuilt images.
# Runtime still falls back to /app/data/embeddings-cache when this cache is not present.
ARG AGENTARK_PREFETCH_LOCAL_EMBEDDINGS=true
RUN --mount=type=cache,id=agentark-cargo-target,target=/app/target \
    --mount=type=cache,id=agentark-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=agentark-local-embeddings-cache,target=/tmp/agentark-embeddings-cache \
    mkdir -p /app/prebuilt-embeddings-cache /tmp/agentark-embeddings-cache && \
    if [ "${AGENTARK_PREFETCH_LOCAL_EMBEDDINGS}" = "true" ]; then \
        target/release/prefetch_embeddings /tmp/agentark-embeddings-cache && \
        cp -a /tmp/agentark-embeddings-cache/. /app/prebuilt-embeddings-cache/; \
    else \
        mkdir -p /app/prebuilt-embeddings-cache; \
    fi

# -- Stage 2: Frontend build --
FROM node:25-slim AS frontend-builder
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN --mount=type=cache,id=agentark-frontend-npm,target=/root/.npm \
    npm pkg delete devDependencies.@rollup/rollup-win32-x64-msvc 2>/dev/null; npm ci
ARG FRONTEND_CACHEBUST=0
COPY frontend/src ./src
COPY frontend/index.html frontend/tsconfig.json frontend/tsconfig.node.json frontend/vite.config.ts ./
RUN npm run build

# -- Stage 3: Node.js bridges build --
# Build node_modules here (git available), then copy only the result to runtime
FROM node:25-slim AS node-builder

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

# -- Stage 4: Minimal runtime --
# Keep runtime on the same Debian family so the final binary sees the same
# glibc generation it was linked against in the builder stage.
FROM node:25-trixie-slim

ARG INSTALL_PLAYWRIGHT_RUNTIME=false
ARG INSTALL_TAILSCALE=false
ARG INSTALL_CLOUDFLARED=false
ARG INSTALL_LIGHTPANDA=true
ARG INSTALL_GWS=false
ARG INSTALL_DOCKER_CLI=true
ARG INSTALL_OLLAMA_CLI=false

RUN set -eux; \
    apt_packages="ca-certificates curl gosu git python3 python3-pip python3-venv"; \
    if [ "${INSTALL_PLAYWRIGHT_RUNTIME}" = "true" ]; then \
        apt_packages="${apt_packages} chromium"; \
    fi; \
    if [ "${INSTALL_DOCKER_CLI}" = "true" ]; then \
        apt_packages="${apt_packages} docker.io"; \
    fi; \
    if [ "${INSTALL_OLLAMA_CLI}" = "true" ]; then \
        apt_packages="${apt_packages} zstd"; \
    fi; \
    apt-get update; \
    apt-get install -y --no-install-recommends ${apt_packages}; \
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
        curl -fsSL --retry 3 \
            "https://github.com/cloudflare/cloudflared/releases/download/${CLOUDFLARED_VERSION}/cloudflared-linux-amd64" \
            -o /usr/local/bin/cloudflared && \
        chmod +x /usr/local/bin/cloudflared; \
    fi

# Download Lightpanda for fast headless content extraction (~6MB vs ~1.5GB Chromium)
# Used as fast-path for http_get, web search scraping, and research content fetching.
# Playwright remains for screenshots and complex SPA interaction.
ARG LIGHTPANDA_RELEASE=nightly
RUN if [ "${INSTALL_LIGHTPANDA}" = "true" ]; then \
        curl -fsSL --retry 3 \
            "https://github.com/lightpanda-io/browser/releases/download/${LIGHTPANDA_RELEASE}/lightpanda-x86_64-linux" \
            -o /usr/local/bin/lightpanda && \
        chmod +x /usr/local/bin/lightpanda; \
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
COPY --from=builder --chown=agent:agent /app/prebuilt-embeddings-cache /app/prebuilt-embeddings-cache

# Copy assets directly from build context (not part of Rust compilation)
COPY --chown=agent:agent config /app/config
COPY --chown=agent:agent skills /app/skills
COPY --chown=agent:agent assets /app/assets
RUN test -d /app/skills && find /app/skills -mindepth 2 -maxdepth 2 -name SKILL.md | grep -q .
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
# Default bridge URL for in-container Playwright service
ENV PLAYWRIGHT_BRIDGE_URL=http://127.0.0.1:3100
# Secure logging: suppress SQLx queries to prevent sensitive data exposure
ENV RUST_LOG=info,sqlx::query=warn,sea_orm=warn,hyper=warn,reqwest=warn

# Expose HTTP API port
EXPOSE 8990

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD python3 -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8990/health', timeout=5)" || exit 1

# Run with entrypoint script that checks for volume mounts
ENTRYPOINT ["/app/docker-entrypoint.sh"]
CMD ["--headless"]
