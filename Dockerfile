# =============================================================================
# AgentArk - AI Agent Docker Image
# =============================================================================
#
# RECOMMENDED: Use docker-compose for automatic data persistence
#
#   ./scripts/start.sh  (Linux/Mac)
#   scripts/start.bat   (Windows)
#   docker-compose up -d --build
#
# Your data (conversations, skills, settings) is automatically preserved
# across rebuilds when using docker-compose.
#
# =============================================================================
# MANUAL DOCKER RUN (must include volumes to preserve data):
#
#   docker run -d -p 8990:8990 \
#     -v agentark-data:/app/data \
#     -v agentark-config:/app/config \
#     --name agentark \
#     agentark:latest
#
# WARNING: Running without -v volumes will LOSE YOUR DATA on container removal!
# =============================================================================

# ── Stage 1: Rust build (with BuildKit cache for fast rebuilds) ─────────────
FROM rust:1.92-bookworm AS builder

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Create dummy main to cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Low-memory build: thin LTO + more codegen units to reduce linker peak RAM
# Full LTO needs 3-4GB+ for wasmtime; thin LTO fits in 2GB RAM + swap
ENV CARGO_BUILD_JOBS=2
ENV CARGO_PROFILE_RELEASE_LTO=thin
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=4

# Build dependencies with cache mount (survives across docker builds)
RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release && rm -rf src

# Copy source + assets (logo.svg is included at compile time via include_str!)
COPY src ./src
COPY assets ./assets

# Build for release with cache mount, then copy binary out of cache
RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    touch src/main.rs && cargo build --release && \
    cp target/release/agentark /app/agentark-bin

# ── Stage 2: Frontend build ──────────────────────────────────────────────────
FROM node:20-slim AS frontend-builder
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm pkg delete devDependencies.@rollup/rollup-win32-x64-msvc 2>/dev/null; npm ci
COPY frontend/src ./src
COPY frontend/index.html frontend/tsconfig.json frontend/tsconfig.node.json frontend/vite.config.ts ./
RUN npm run build

# ── Stage 3: Node.js bridges build ───────────────────────────────────────────
# Build node_modules here (git available), then copy only the result to runtime
FROM node:20-slim AS node-builder

RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /bridge/whatsapp-bridge
COPY services/whatsapp-bridge/package.json services/whatsapp-bridge/package-lock.json ./
RUN printf '[url "https://github.com/"]\n\tinsteadOf = ssh://git@github.com/\n\tinsteadOf = git@github.com:\n' > /root/.gitconfig && \
    npm ci --omit=dev && \
    npm cache clean --force && \
    rm -rf /root/.npm /root/.gitconfig /tmp/*
COPY services/whatsapp-bridge/index.js ./

# Playwright bridge (skip browser download; runtime image provides browsers)
WORKDIR /bridge/playwright-bridge
ENV PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1
COPY services/playwright-bridge/package.json services/playwright-bridge/package-lock.json ./
RUN npm ci --omit=dev && \
    npm cache clean --force && \
    rm -rf /root/.npm /tmp/*
COPY services/playwright-bridge/index.js ./

# Remotion video template (pre-install node_modules for fast renders)
WORKDIR /bridge/remotion-template
COPY services/remotion-template/package.json services/remotion-template/package-lock.json ./
RUN npm ci --omit=dev 2>/dev/null && \
    npm cache clean --force && \
    rm -rf /root/.npm /tmp/*
COPY services/remotion-template/src ./src
COPY services/remotion-template/tsconfig.json services/remotion-template/remotion.config.ts ./

# ── Stage 4: Minimal runtime ────────────────────────────────────────────────
FROM mcr.microsoft.com/playwright:v1.58.2-noble

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    curl \
    docker.io \
    docker-compose \
    gosu \
    ffmpeg \
    git \
    python3 \
    python3-pip \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Create non-root user + all directories in one layer
RUN useradd -m agent && \
    mkdir -p /app/data /app/data/skills /app/data/whatsapp-auth /app/config /app/whatsapp-bridge /app/playwright-bridge /app/mem0-bridge && \
    chown -R agent:agent /app

# Install Mem0 Python dependencies (runs as root, before dropping privileges)
COPY services/mem0-bridge/requirements.txt /app/mem0-bridge/
RUN pip3 install --no-cache-dir --break-system-packages -r /app/mem0-bridge/requirements.txt

# Copy Mem0 bridge app
COPY --chown=agent:agent services/mem0-bridge/app.py /app/mem0-bridge/


# Download cloudflared for built-in tunnel support (zero-friction remote access)
# Pinned version for reproducible builds — update deliberately after testing.
ADD https://github.com/cloudflare/cloudflared/releases/download/2026.2.0/cloudflared-linux-amd64 /usr/local/bin/cloudflared
RUN chmod +x /usr/local/bin/cloudflared

# Copy pre-built bridges with node_modules (owned by agent)
COPY --from=node-builder --chown=agent:agent /bridge/whatsapp-bridge /app/whatsapp-bridge
COPY --from=node-builder --chown=agent:agent /bridge/playwright-bridge /app/playwright-bridge

# Copy Remotion template with pre-installed node_modules (for video generation)
COPY --from=node-builder --chown=agent:agent /bridge/remotion-template /app/services/remotion-template

# Copy AgentArk binary from builder
COPY --from=builder --chown=agent:agent /app/agentark-bin /app/agentark

# Copy assets directly from build context (not part of Rust compilation)
COPY --chown=agent:agent config /app/config
COPY --chown=agent:agent skills /app/skills
COPY --chown=agent:agent assets /app/assets
# Copy frontend assets (built in Docker, not from host)
COPY --from=frontend-builder --chown=agent:agent /app/frontend/dist /app/frontend/dist
# frontend/legacy is optional (static fallback assets)
RUN mkdir -p /app/frontend/legacy && chown agent:agent /app/frontend/legacy

# Copy entrypoint script (fix Windows CRLF line endings)
COPY --chown=agent:agent docker-entrypoint.sh /app/
RUN sed -i 's/\r$//' /app/docker-entrypoint.sh && chmod +x /app/docker-entrypoint.sh

# Start as root — entrypoint will fix docker socket perms then drop to agent

# Environment
ENV AGENTARK_CONFIG=/app/config
ENV AGENTARK_DATA=/app/data
# Playwright browsers are preinstalled in the base image
ENV PLAYWRIGHT_BROWSERS_PATH=/ms-playwright
# Default bridge URL for in-container Playwright service
ENV PLAYWRIGHT_BRIDGE_URL=http://127.0.0.1:3100
# Secure logging: suppress SQLx queries to prevent sensitive data exposure
ENV RUST_LOG=info,sqlx::query=warn,sea_orm=warn,hyper=warn,reqwest=warn

# Expose HTTP API port
EXPOSE 8990

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8990/health || exit 1

# Run with entrypoint script that checks for volume mounts
ENTRYPOINT ["/app/docker-entrypoint.sh"]
CMD ["--headless"]
