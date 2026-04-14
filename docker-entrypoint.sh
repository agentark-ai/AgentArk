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
HEALTH_WATCHDOG_PID=""
XVFB_PID=""
OPENBOX_PID=""
X11VNC_PID=""
NOVNC_PID=""

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
    stop_playwright_display_stack
}

stop_tracked_process() {
    local pid="${1:-}"
    if [ -n "$pid" ] && kill -0 "$pid" >/dev/null 2>&1; then
        kill "$pid" >/dev/null 2>&1 || true
        wait "$pid" >/dev/null 2>&1 || true
    fi
}

cleanup_x_display_state() {
    local display_name="${1:-:99}"
    local display_number="${display_name#:}"
    display_number="${display_number%%.*}"
    local lock_path="/tmp/.X${display_number}-lock"
    local socket_path="/tmp/.X11-unix/X${display_number}"

    if command -v pgrep >/dev/null 2>&1 && pgrep -f "Xvfb ${display_name}" >/dev/null 2>&1; then
        return
    fi

    rm -f "$lock_path" "$socket_path" >/dev/null 2>&1 || true
}

stop_playwright_display_stack() {
    stop_tracked_process "$NOVNC_PID"
    stop_tracked_process "$X11VNC_PID"
    stop_tracked_process "$OPENBOX_PID"
    stop_tracked_process "$XVFB_PID"
    NOVNC_PID=""
    X11VNC_PID=""
    OPENBOX_PID=""
    XVFB_PID=""
    cleanup_x_display_state "${DISPLAY:-:99}"
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

# Run setup as root
setup_docker_socket
check_volume_mount
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
start_playwright_display_stack() {
    local role
    role=$(normalized_stack_role)

    if ! should_start_playwright_bridge; then
        return
    fi

    if truthy_env "${PLAYWRIGHT_HEADLESS:-false}"; then
        echo -e "${GREEN}Skipping Playwright live display stack because PLAYWRIGHT_HEADLESS is enabled${NC}"
        return
    fi

    if ! command -v Xvfb >/dev/null 2>&1 || ! command -v x11vnc >/dev/null 2>&1 || ! command -v websockify >/dev/null 2>&1; then
        echo -e "${YELLOW}Playwright live display stack not available (Xvfb/x11vnc/websockify missing)${NC}"
        return
    fi

    export DISPLAY=${DISPLAY:-:99}
    local vnc_port=${PLAYWRIGHT_VNC_PORT:-5900}
    local novnc_port=${PLAYWRIGHT_LIVE_VIEW_INTERNAL_PORT:-6080}

    cleanup_x_display_state "$DISPLAY"

    echo -e "${GREEN}Starting Playwright live display stack on ${DISPLAY}...${NC}"
    Xvfb "$DISPLAY" -screen 0 1440x960x24 -ac +extension RANDR >/tmp/agentark-xvfb.log 2>&1 &
    XVFB_PID=$!
    track_child "$XVFB_PID"
    sleep 1

    if ! kill -0 "$XVFB_PID" >/dev/null 2>&1; then
        wait "$XVFB_PID" >/dev/null 2>&1 || true
        XVFB_PID=""
        cleanup_x_display_state "$DISPLAY"
        echo -e "${YELLOW}Playwright live display stack disabled because Xvfb failed to start on ${DISPLAY}.${NC}"
        return
    fi

    if command -v openbox >/dev/null 2>&1; then
        DISPLAY="$DISPLAY" openbox >/tmp/agentark-openbox.log 2>&1 &
        OPENBOX_PID=$!
        track_child "$OPENBOX_PID"
    fi

    x11vnc -display "$DISPLAY" -rfbport "$vnc_port" -localhost -forever -shared -nopw -xkb >/tmp/agentark-x11vnc.log 2>&1 &
    X11VNC_PID=$!
    track_child "$X11VNC_PID"
    sleep 1

    if ! kill -0 "$X11VNC_PID" >/dev/null 2>&1; then
        wait "$X11VNC_PID" >/dev/null 2>&1 || true
        X11VNC_PID=""
        stop_playwright_display_stack
        echo -e "${YELLOW}Playwright live display stack disabled because x11vnc could not attach to ${DISPLAY}.${NC}"
        return
    fi

    if [ -d /usr/share/novnc ]; then
        websockify --web=/usr/share/novnc/ 0.0.0.0:"$novnc_port" 127.0.0.1:"$vnc_port" >/tmp/agentark-novnc.log 2>&1 &
        NOVNC_PID=$!
        track_child "$NOVNC_PID"
        sleep 1
        if ! kill -0 "$NOVNC_PID" >/dev/null 2>&1; then
            wait "$NOVNC_PID" >/dev/null 2>&1 || true
            NOVNC_PID=""
            echo -e "${YELLOW}Playwright live handoff UI could not be published on localhost:${novnc_port}.${NC}"
            return
        fi
        echo -e "${GREEN}Playwright live handoff UI available on localhost:${novnc_port}${NC}"
    else
        echo -e "${YELLOW}noVNC assets not found under /usr/share/novnc; live browser handoff UI will be unavailable${NC}"
    fi
}

start_playwright_display_stack

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
        PLAYWRIGHT_HEADLESS=${PLAYWRIGHT_HEADLESS:-false} \
        PLAYWRIGHT_LIVE_VIEW_PORT=${AGENTARK_BROWSER_HANDOFF_PUBLIC_PORT:-${PLAYWRIGHT_LIVE_VIEW_PORT:-6080}} \
        PLAYWRIGHT_LIVE_VIEW_PATH=${PLAYWRIGHT_LIVE_VIEW_PATH:-/vnc.html?autoconnect=1&resize=remote&path=websockify} \
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

default_health_watchdog_url() {
    local role
    role=$(normalized_stack_role)
    case "$role" in
        executor)
            echo "http://127.0.0.1:8991/health"
            ;;
        workspace)
            echo "http://127.0.0.1:8992/health"
            ;;
        *)
            echo "http://127.0.0.1:8990/health"
            ;;
    esac
}

start_health_watchdog() {
    local role
    role=$(normalized_stack_role)

    # Docker already has an outer healthcheck and restart policy for this image.
    # Keep the in-process watchdog opt-in so a transient startup stall does not
    # self-terminate the service.
    if [ -z "${AGENTARK_SELF_WATCHDOG:-}" ]; then
        return
    fi
    truthy_env "$AGENTARK_SELF_WATCHDOG" || return

    local url=${AGENTARK_SELF_WATCHDOG_URL:-$(default_health_watchdog_url)}
    local interval=${AGENTARK_SELF_WATCHDOG_INTERVAL_SECS:-15}
    local timeout=${AGENTARK_SELF_WATCHDOG_TIMEOUT_SECS:-5}
    local max_failures=${AGENTARK_SELF_WATCHDOG_MAX_FAILURES:-3}
    local initial_delay=${AGENTARK_SELF_WATCHDOG_INITIAL_DELAY_SECS:-30}

    (
        failures=0
        sleep "$initial_delay"
        while true; do
            if ! kill -0 "$MAIN_PID" >/dev/null 2>&1; then
                exit 0
            fi

            if python3 -c "import urllib.request; urllib.request.urlopen('${url}', timeout=${timeout})" >/dev/null 2>&1; then
                failures=0
            else
                failures=$((failures + 1))
                echo -e "${YELLOW}Health watchdog: local probe failed (${failures}/${max_failures}) for ${url}.${NC}"
            fi

            if [ "$failures" -ge "$max_failures" ]; then
                echo -e "${RED}Health watchdog: ${role:-control} service stopped answering ${url}; terminating AgentArk so Docker can restart it.${NC}"
                kill "$MAIN_PID" >/dev/null 2>&1 || true
                sleep 10
                kill -9 "$MAIN_PID" >/dev/null 2>&1 || true
                exit 0
            fi

            sleep "$interval"
        done
    ) &
    HEALTH_WATCHDOG_PID=$!
    track_child "$HEALTH_WATCHDOG_PID"
}

# Drop privileges to 'agent' user and start the app under supervision
gosu agent /app/agentark "$@" &
MAIN_PID=$!
track_child "$MAIN_PID"
start_health_watchdog

while true; do
    if ! kill -0 "$MAIN_PID" >/dev/null 2>&1; then
        wait "$MAIN_PID"
        exit $?
    fi

    handle_optional_service_exit "Tailscale daemon" start_tailscale_daemon TAILSCALE_PID TAILSCALE_RESTARTS
    handle_optional_service_exit "Playwright bridge" start_playwright_bridge PLAYWRIGHT_PID PLAYWRIGHT_RESTARTS

    sleep 5
done
