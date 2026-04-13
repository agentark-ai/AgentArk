#!/bin/bash
# AgentArk Docker Entrypoint
# - Fixes Docker socket permissions for sandboxed code execution
# - Drops privileges to 'agent' user before starting the app

set -e

# Colors for output
RED='\033[0;31m'
YELLOW='\033[1;33m'
GREEN='\033[0;32m'
NC='\033[0m'
CHILD_PIDS=""
TAILSCALE_RESTARTS=0
PLAYWRIGHT_RESTARTS=0
MAX_OPTIONAL_SERVICE_RESTARTS=${AGENTARK_OPTIONAL_SERVICE_RESTARTS:-1}

track_child() {
    if [ -n "${1:-}" ]; then
        CHILD_PIDS="$CHILD_PIDS $1"
    fi
}

cleanup_children() {
    for pid in $CHILD_PIDS; do
        if kill -0 "$pid" >/dev/null 2>&1; then
            kill "$pid" >/dev/null 2>&1 || true
        fi
    done
}

normalized_stack_role() {
    local role
    role=$(printf '%s' "${AGENTARK_STACK_ROLE:-}" | tr '[:upper:]' '[:lower:]')
    case "$role" in
        control-plane)
            echo "control"
            ;;
        *)
            echo "$role"
            ;;
    esac
}

truthy_env() {
    case "$(printf '%s' "${1:-}" | tr '[:upper:]' '[:lower:]')" in
        1|true|yes|on)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

falsy_env() {
    case "$(printf '%s' "${1:-}" | tr '[:upper:]' '[:lower:]')" in
        0|false|no|off)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

role_uses_local_docker() {
    local role
    role=$(normalized_stack_role)
    [ -z "$role" ] || [ "$role" = "executor" ]
}

should_start_tailscale_daemon() {
    if [ -n "${AGENTARK_START_TAILSCALE_DAEMON:-}" ]; then
        truthy_env "$AGENTARK_START_TAILSCALE_DAEMON" && return 0
        falsy_env "$AGENTARK_START_TAILSCALE_DAEMON" && return 1
    fi

    if [ -n "${AGENTARK_TUNNEL:-}" ]; then
        truthy_env "$AGENTARK_TUNNEL" && return 0
        falsy_env "$AGENTARK_TUNNEL" && return 1
    fi

    local role
    role=$(normalized_stack_role)
    [ -z "$role" ] || [ "$role" = "control" ]
}

should_start_playwright_bridge() {
    if [ -n "${AGENTARK_START_PLAYWRIGHT_BRIDGE:-}" ]; then
        truthy_env "$AGENTARK_START_PLAYWRIGHT_BRIDGE" && return 0
        falsy_env "$AGENTARK_START_PLAYWRIGHT_BRIDGE" && return 1
    fi

    local role
    role=$(normalized_stack_role)
    [ -z "$role" ] || [ "$role" = "control" ]
}

print_startup_banner() {
    local role
    role=$(normalized_stack_role)

    echo ""
    echo "============================================"
    case "$role" in
        executor)
            echo "  AgentArk Executor Starting..."
            echo "  Internal Executor API: 0.0.0.0:8991"
            ;;
        workspace)
            echo "  AgentArk Workspace Service Starting..."
            echo "  Internal Workspace API: 0.0.0.0:8992"
            ;;
        *)
            echo "  AgentArk Starting..."
            echo "  Web UI: http://localhost:8990"
            ;;
    esac
    echo "============================================"
    echo ""
}

trap cleanup_children EXIT INT TERM

# Ensure directories exist with proper permissions
ensure_directories() {
    # User-owned layer: release updates may recreate image files, but these
    # mounted data/config paths must persist across normal rebuilds.
    mkdir -p /app/data/skills 2>/dev/null || true
    mkdir -p /app/data/tailscale 2>/dev/null || true
    mkdir -p /app/config 2>/dev/null || true
    chown -R agent:agent /app/data /app/config 2>/dev/null || true
}

load_internal_service_tokens() {
    TOKENS_FILE=${AGENTARK_INTERNAL_TOKENS_FILE:-/app/config/internal-service-tokens.env}
    LOCK_DIR="${TOKENS_FILE}.lock"
    LOCK_WAIT_TICKS=0

    if [ -f "$TOKENS_FILE" ]; then
        set -a
        . "$TOKENS_FILE"
        set +a
        chmod 600 "$TOKENS_FILE" 2>/dev/null || true
        chown root:root "$TOKENS_FILE" 2>/dev/null || true
    fi

    if [ -n "${AGENTARK_EXECUTOR_TOKEN:-}" ] && [ -n "${AGENTARK_WORKSPACE_TOKEN:-}" ]; then
        return
    fi

    while ! mkdir "$LOCK_DIR" 2>/dev/null; do
        LOCK_WAIT_TICKS=$((LOCK_WAIT_TICKS + 1))
        if [ "$LOCK_WAIT_TICKS" -ge 300 ]; then
            echo -e "${RED}Timed out waiting for the internal token bootstrap lock.${NC}"
            exit 1
        fi
        sleep 0.1
    done

    if [ -f "$TOKENS_FILE" ]; then
        set -a
        . "$TOKENS_FILE"
        set +a
        chmod 600 "$TOKENS_FILE" 2>/dev/null || true
        chown root:root "$TOKENS_FILE" 2>/dev/null || true
    fi

    if [ -z "${AGENTARK_EXECUTOR_TOKEN:-}" ] || [ -z "${AGENTARK_WORKSPACE_TOKEN:-}" ]; then
        umask 077
        EXECUTOR_TOKEN=$(python3 -c "import secrets; print(secrets.token_hex(32))")
        WORKSPACE_TOKEN=$(python3 -c "import secrets; print(secrets.token_hex(32))")
        TMP_FILE="${TOKENS_FILE}.tmp.$$"
        cat > "$TMP_FILE" <<EOF
AGENTARK_EXECUTOR_TOKEN=$EXECUTOR_TOKEN
AGENTARK_WORKSPACE_TOKEN=$WORKSPACE_TOKEN
EOF
        chmod 600 "$TMP_FILE"
        mv "$TMP_FILE" "$TOKENS_FILE"
        chown root:root "$TOKENS_FILE" 2>/dev/null || true
        export AGENTARK_EXECUTOR_TOKEN="$EXECUTOR_TOKEN"
        export AGENTARK_WORKSPACE_TOKEN="$WORKSPACE_TOKEN"
        echo -e "${GREEN}Generated internal service tokens for this install.${NC}"
    fi

    rmdir "$LOCK_DIR" 2>/dev/null || true

    if [ -f "$TOKENS_FILE" ]; then
        set -a
        . "$TOKENS_FILE"
        set +a
        chmod 600 "$TOKENS_FILE" 2>/dev/null || true
        chown root:root "$TOKENS_FILE" 2>/dev/null || true
    fi
}

# Fix Docker socket permissions so 'agent' can spawn sandboxed containers
setup_docker_socket() {
    local role
    role=$(normalized_stack_role)

    if [ -S /var/run/docker.sock ]; then
        # Direct socket mount - fix permissions
        DOCKER_GID=$(stat -c '%g' /var/run/docker.sock)
        if ! getent group "$DOCKER_GID" >/dev/null 2>&1; then
            groupadd -g "$DOCKER_GID" dockerhost 2>/dev/null || true
        fi
        DOCKER_GROUP=$(getent group "$DOCKER_GID" | cut -d: -f1)
        usermod -aG "$DOCKER_GROUP" agent 2>/dev/null || true
        echo -e "${GREEN}Docker socket available - sandboxed code execution enabled${NC}"
    elif [ -n "${DOCKER_HOST:-}" ]; then
        # TCP proxy (docker-socket-proxy) - no socket permissions needed
        echo -e "${GREEN}Docker available via proxy ($DOCKER_HOST) - sandboxed code execution enabled${NC}"
    elif role_uses_local_docker; then
        echo -e "${YELLOW}Docker not available - sandboxed code execution is unavailable${NC}"
    elif [ "$role" = "control" ]; then
        echo -e "${GREEN}Docker socket not mounted for control role - sandboxed execution is delegated to the executor service${NC}"
    elif [ "$role" = "workspace" ]; then
        echo -e "${GREEN}Docker socket not mounted for workspace role - this service does not execute sandboxed code${NC}"
    else
        echo -e "${GREEN}Docker socket not mounted for this service role${NC}"
    fi
}

# Check if data volume is mounted
check_volume_mount() {
    ensure_directories

    if [ -z "$(ls -A /app/data 2>/dev/null)" ]; then
        if [ ! -f /app/data/.volume_initialized ]; then
            echo ""
            echo -e "${YELLOW}============================================${NC}"
            echo -e "${YELLOW}  FIRST RUN DETECTED${NC}"
            echo -e "${YELLOW}============================================${NC}"
            echo ""
            echo "Initializing data directory..."
            touch /app/data/.volume_initialized
            chown agent:agent /app/data/.volume_initialized
            echo ""
            echo -e "${GREEN}Your data will be stored in Docker volumes.${NC}"
            echo -e "${GREEN}It will persist across container rebuilds.${NC}"
            echo ""
        fi
    else
        echo -e "${GREEN}Existing data found - your conversations and skills are preserved.${NC}"
    fi
}

check_bundled_skills() {
    if [ ! -d /app/skills ]; then
        echo -e "${RED}Bundled skills directory /app/skills is missing. This image is incomplete.${NC}"
        return
    fi

    BUNDLED_SKILL_COUNT=$(find /app/skills -mindepth 2 -maxdepth 2 -name SKILL.md 2>/dev/null | wc -l | tr -d ' ')
    if [ "${BUNDLED_SKILL_COUNT:-0}" -eq 0 ]; then
        echo -e "${YELLOW}No bundled SKILL.md files are currently present under /app/skills. They may have been removed for this install or omitted from the image.${NC}"
    else
        echo -e "${GREEN}Bundled skills available - ${BUNDLED_SKILL_COUNT} SKILL.md files found under /app/skills${NC}"
    fi
}

# Run setup as root
setup_docker_socket
check_volume_mount
check_bundled_skills
load_internal_service_tokens

# WhatsApp bridge is bundled in the full image and managed by the AgentArk backend on demand
# when WhatsApp Baileys runs in bundled bridge mode. Cloud API mode does not start it.

# Confirm Docker secret is available for direct app reads (if present)
if [ -f /run/secrets/agentark_master_key ]; then
    echo -e "${GREEN}Docker secret found - application will read the encryption secret directly from /run/secrets${NC}"
fi

# Print startup banner
print_startup_banner

start_tailscale_daemon() {
    local role
    role=$(normalized_stack_role)
    TAILSCALE_PID=""

    # Split-stack containers share /app/data, so only the control/default role
    # should own the shared Tailscale state directory unless explicitly
    # overridden.
    if ! should_start_tailscale_daemon; then
        if [ -n "$role" ]; then
            echo -e "${GREEN}Skipping Tailscale daemon for ${role} role${NC}"
        else
            echo -e "${GREEN}Skipping Tailscale daemon because remote access is disabled${NC}"
        fi
        return
    fi

    export TS_STATE_DIR=${TS_STATE_DIR:-/app/data/tailscale}
    export TS_SOCKET=${TS_SOCKET:-/app/data/tailscale/tailscaled.sock}
    export TS_USERSPACE=${TS_USERSPACE:-true}

    if command -v tailscaled >/dev/null 2>&1 && command -v tailscale >/dev/null 2>&1; then
        mkdir -p "$TS_STATE_DIR"
        chown -R agent:agent "$TS_STATE_DIR" 2>/dev/null || true
        rm -f "$TS_SOCKET" 2>/dev/null || true
        echo -e "${GREEN}Starting Tailscale daemon (userspace, persistent state)...${NC}"
        gosu agent tailscaled \
            --statedir="$TS_STATE_DIR" \
            --socket="$TS_SOCKET" \
            --tun=userspace-networking &
        TAILSCALE_PID=$!
        track_child "$TAILSCALE_PID"

        for _ in $(seq 1 20); do
            if [ -S "$TS_SOCKET" ]; then
                echo -e "${GREEN}Tailscale daemon started (PID: $TAILSCALE_PID)${NC}"
                return
            fi
            sleep 1
        done

        echo -e "${YELLOW}Tailscale daemon did not expose its socket in time; Tailscale tunnel actions may fail.${NC}"
    else
        echo -e "${YELLOW}Tailscale runtime not installed; Tailscale tunnel providers will stay unavailable.${NC}"
    fi
}

start_tailscale_daemon

# Start Playwright bridge in background (localhost-only)
start_playwright_bridge() {
    local role
    role=$(normalized_stack_role)
    PLAYWRIGHT_PID=""

    if ! should_start_playwright_bridge; then
        if [ -n "$role" ]; then
            echo -e "${GREEN}Skipping Playwright bridge for ${role} role${NC}"
        fi
        return
    fi

    if ! command -v node >/dev/null 2>&1 || [ ! -f /app/bridges/playwright-bridge/index.js ] || [ ! -d /app/bridges/playwright-bridge/node_modules ]; then
        echo -e "${YELLOW}Playwright bridge not available (Node.js or bridge dependencies missing)${NC}"
        return
    fi

    if [ -n "${PLAYWRIGHT_EXECUTABLE_PATH:-}" ] && [ ! -x "${PLAYWRIGHT_EXECUTABLE_PATH}" ]; then
        if [ -z "$(find "${PLAYWRIGHT_BROWSERS_PATH:-/nonexistent}" -mindepth 1 -maxdepth 1 2>/dev/null | head -n 1)" ]; then
            echo -e "${YELLOW}Playwright bridge not available (no Chromium binary or bundled Playwright browsers found)${NC}"
            return
        fi
    fi

    if command -v node >/dev/null 2>&1 && [ -f /app/bridges/playwright-bridge/index.js ]; then
        echo -e "${GREEN}Starting Playwright bridge (localhost:3100)...${NC}"
        PLAYWRIGHT_BROWSERS_PATH=${PLAYWRIGHT_BROWSERS_PATH:-/app/.playwright-browsers} \
        PLAYWRIGHT_EXECUTABLE_PATH=${PLAYWRIGHT_EXECUTABLE_PATH:-} \
        PORT=${PLAYWRIGHT_BRIDGE_PORT:-3100} \
        PLAYWRIGHT_BRIDGE_HOST=${PLAYWRIGHT_BRIDGE_HOST:-127.0.0.1} \
        gosu agent node /app/bridges/playwright-bridge/index.js &
        PLAYWRIGHT_PID=$!
        track_child "$PLAYWRIGHT_PID"
        echo -e "${GREEN}Playwright bridge started (PID: $PLAYWRIGHT_PID)${NC}"
    fi
}

start_playwright_bridge

handle_optional_service_exit() {
    local name="$1"
    local restart_fn="$2"
    local -n pid_ref="$3"
    local -n restart_ref="$4"

    if [ -z "${pid_ref:-}" ] || kill -0 "$pid_ref" >/dev/null 2>&1; then
        return
    fi

    local pid="$pid_ref"
    local status=0
    wait "$pid" >/dev/null 2>&1 || status=$?
    pid_ref=""

    echo -e "${YELLOW}${name} exited unexpectedly (PID: $pid, status: $status). ${NC}"
    echo -e "${YELLOW}AgentArk will continue without ${name}.${NC}"

    if [ "$restart_ref" -lt "$MAX_OPTIONAL_SERVICE_RESTARTS" ]; then
        restart_ref=$((restart_ref + 1))
        echo -e "${YELLOW}Restarting optional service '${name}' (${restart_ref}/${MAX_OPTIONAL_SERVICE_RESTARTS})...${NC}"
        "$restart_fn"
    else
        echo -e "${YELLOW}${name} reached the restart limit; leaving it disabled for this container session.${NC}"
    fi
}

# WhatsApp bridge: started by AgentArk on demand for Baileys bundled bridge mode only

# Drop privileges to 'agent' user and start the app under supervision
gosu agent /app/agentark "$@" &
MAIN_PID=$!
track_child "$MAIN_PID"

while true; do
    if ! kill -0 "$MAIN_PID" >/dev/null 2>&1; then
        wait "$MAIN_PID"
        exit $?
    fi

    handle_optional_service_exit "Tailscale daemon" start_tailscale_daemon TAILSCALE_PID TAILSCALE_RESTARTS
    handle_optional_service_exit "Playwright bridge" start_playwright_bridge PLAYWRIGHT_PID PLAYWRIGHT_RESTARTS

    sleep 5
done
