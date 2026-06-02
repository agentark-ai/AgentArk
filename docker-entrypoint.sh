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
        embeddings)
            echo "  AgentArk Embeddings Sidecar Starting..."
            echo "  Internal Embeddings API: ${AGENTARK_EMBEDDINGS_BIND:-0.0.0.0:8993}"
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

# Assemble AGENTARK_DATABASE_URL from the password file written by the
# Postgres service startup wrapper (see docker-compose.yml). The password is never
# baked into this image — it lives in the agentark-secrets Docker volume,
# is read only at runtime, and is never logged.
#
# Precedence:
#   1. AGENTARK_DATABASE_URL already set (external Postgres override) — use as-is
#   2. AGENTARK_POSTGRES_PASSWORD_FILE readable — assemble URL from file + host/port/user/db env
#   3. AGENTARK_POSTGRES_PASSWORD set in env (CI / custom setups) — assemble from env
#   4. No DB role (workspace) — skip; DB is not required
#   5. DB role but no source — hard-fail loudly rather than silently defaulting
build_database_url_from_secret() {
    if [ -n "${AGENTARK_DATABASE_URL:-}" ]; then
        return 0
    fi

    local pg_user="${AGENTARK_POSTGRES_USER:-agentark}"
    local pg_db="${AGENTARK_POSTGRES_DB:-agentark}"
    local pg_host="${AGENTARK_POSTGRES_HOST:-postgres}"
    local pg_port="${AGENTARK_POSTGRES_PORT:-5432}"
    local pg_pw=""

    if [ -n "${AGENTARK_POSTGRES_PASSWORD_FILE:-}" ] && [ -r "${AGENTARK_POSTGRES_PASSWORD_FILE}" ] && [ -s "${AGENTARK_POSTGRES_PASSWORD_FILE}" ]; then
        pg_pw="$(cat "${AGENTARK_POSTGRES_PASSWORD_FILE}")"
    elif [ -n "${AGENTARK_POSTGRES_PASSWORD:-}" ]; then
        pg_pw="${AGENTARK_POSTGRES_PASSWORD}"
    fi

    if [ -n "$pg_pw" ]; then
        AGENTARK_DATABASE_URL="$(AGENTARK_PG_PASSWORD_VALUE="$pg_pw" python3 - <<'PY'
import os
import urllib.parse

user = os.environ.get("AGENTARK_POSTGRES_USER", "agentark")
password = os.environ["AGENTARK_PG_PASSWORD_VALUE"]
host = os.environ.get("AGENTARK_POSTGRES_HOST", "postgres")
port = os.environ.get("AGENTARK_POSTGRES_PORT", "5432")
database = os.environ.get("AGENTARK_POSTGRES_DB", "agentark")

print(
    "postgres://{user}:{password}@{host}:{port}/{database}".format(
        user=urllib.parse.quote(user, safe=""),
        password=urllib.parse.quote(password, safe=""),
        host=host,
        port=port,
        database=urllib.parse.quote(database, safe=""),
    )
)
PY
)"
        export AGENTARK_DATABASE_URL
        unset pg_pw
        echo -e "${GREEN}Configured Postgres connection from runtime secret.${NC}"
        return 0
    fi

    # Workspace role does not connect to Postgres, so missing DB credentials are fine.
    local role
    role=$(normalized_stack_role)
    if [ "$role" = "workspace" ] || [ "$role" = "embeddings" ]; then
        return 0
    fi

    echo -e "${RED}No Postgres credentials available.${NC}" >&2
    echo -e "${RED}Expected one of: AGENTARK_DATABASE_URL, AGENTARK_POSTGRES_PASSWORD_FILE (mounted from agentark-secrets volume), or AGENTARK_POSTGRES_PASSWORD.${NC}" >&2
    echo -e "${RED}If you ran docker-compose, confirm the Postgres service generated /run/secrets/pg_pw in the 'agentark-secrets' volume.${NC}" >&2
    exit 1
}

ensure_agentark_master_key_secret() {
    local role
    role=$(normalized_stack_role)
    case "$role" in
        control|executor)
            ;;
        *)
            return 0
            ;;
    esac

    if [ ! -s /run/secrets/agentark_master_key ]; then
        echo -e "${RED}Missing install-managed encryption secret at /run/secrets/agentark_master_key.${NC}" >&2
        echo -e "${RED}Start AgentArk with the bundled compose file so the agentark-secrets volume is initialized.${NC}" >&2
        echo -e "${RED}For this pre-release local data, run 'docker compose down -v' before starting again.${NC}" >&2
        exit 1
    fi

    if ! gosu agent sh -c 'test -r /run/secrets/agentark_master_key'; then
        echo -e "${RED}Install-managed encryption secret exists but is not readable by the agent user.${NC}" >&2
        echo -e "${RED}Recreate the bundled compose stack so the agentark-secrets volume is initialized with readable secret permissions.${NC}" >&2
        exit 1
    fi

    echo -e "${GREEN}Install-managed encryption secret available from agentark-secrets volume.${NC}"
}

# Run setup as root
setup_docker_socket
check_volume_mount

if [ "$(normalized_stack_role)" = "embeddings" ]; then
    print_startup_banner
    exec gosu agent /app/agentark-embed-server "$@"
fi

load_internal_service_tokens
build_database_url_from_secret
ensure_agentark_master_key_secret

# WhatsApp bridge is bundled in the full image and managed by the AgentArk backend on demand
# when WhatsApp Baileys runs in bundled bridge mode. Cloud API mode does not start it.

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
    local display_width=${PLAYWRIGHT_DISPLAY_WIDTH:-${PLAYWRIGHT_BROWSER_WIDTH:-1920}}
    local display_height=${PLAYWRIGHT_DISPLAY_HEIGHT:-${PLAYWRIGHT_BROWSER_HEIGHT:-1080}}

    cleanup_x_display_state "$DISPLAY"

    echo -e "${GREEN}Starting Playwright live display stack on ${DISPLAY} (${display_width}x${display_height})...${NC}"
    Xvfb "$DISPLAY" -screen 0 "${display_width}x${display_height}x24" -ac +extension RANDR >/tmp/agentark-xvfb.log 2>&1 &
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
        PLAYWRIGHT_LIVE_VIEW_PATH=${PLAYWRIGHT_LIVE_VIEW_PATH:-/vnc.html?autoconnect=1&resize=scale&path=websockify} \
        PLAYWRIGHT_BROWSER_WIDTH=${PLAYWRIGHT_BROWSER_WIDTH:-1920} \
        PLAYWRIGHT_BROWSER_HEIGHT=${PLAYWRIGHT_BROWSER_HEIGHT:-1080} \
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
        embeddings)
            echo "http://127.0.0.1:8993/health"
            ;;
        *)
            echo "http://127.0.0.1:8990/readiness"
            ;;
    esac
}

health_probe() {
    local url="$1"
    local timeout="$2"
    HEALTH_PROBE_ERROR="$(
        AGENTARK_HEALTH_PROBE_URL="$url" \
        AGENTARK_HEALTH_PROBE_TIMEOUT="$timeout" \
        python3 - <<'PY' 2>&1
import os
import sys
import urllib.request

url = os.environ["AGENTARK_HEALTH_PROBE_URL"]
timeout = float(os.environ.get("AGENTARK_HEALTH_PROBE_TIMEOUT", "10"))

try:
    with urllib.request.urlopen(url, timeout=timeout) as response:
        response.read(1)
        if response.status >= 400:
            raise RuntimeError(f"http_status={response.status}")
except Exception as exc:
    print(f"{type(exc).__name__}: {exc}")
    sys.exit(1)
PY
    )"
}

start_health_watchdog() {
    local role
    role=$(normalized_stack_role)

    local url=${AGENTARK_SELF_WATCHDOG_URL:-$(default_health_watchdog_url)}
    local interval=${AGENTARK_SELF_WATCHDOG_INTERVAL_SECS:-15}
    local timeout=${AGENTARK_SELF_WATCHDOG_TIMEOUT_SECS:-10}
    local max_failures=${AGENTARK_SELF_WATCHDOG_MAX_FAILURES:-6}
    local initial_delay=${AGENTARK_SELF_WATCHDOG_INITIAL_DELAY_SECS:-90}
    local startup_grace=${AGENTARK_SELF_WATCHDOG_STARTUP_GRACE_SECS:-180}

    (
        failures=0
        armed=0
        startup_deadline=$(( $(date +%s) + startup_grace ))
        sleep "$initial_delay"
        while true; do
            if ! kill -0 "$MAIN_PID" >/dev/null 2>&1; then
                exit 0
            fi

            if health_probe "$url" "$timeout"; then
                failures=0
                if [ "$armed" -eq 0 ]; then
                    armed=1
                    echo -e "${GREEN}Health watchdog: armed after first successful probe for ${url}.${NC}"
                fi
            else
                if [ "$armed" -eq 0 ]; then
                    now=$(date +%s)
                    if [ "$now" -lt "$startup_deadline" ]; then
                        remaining=$(( startup_deadline - now ))
                        echo -e "${YELLOW}Health watchdog: startup probe failed for ${url}; error=${HEALTH_PROBE_ERROR:-unknown}; waiting up to ${remaining}s for first healthy response before counting failures.${NC}"
                        sleep "$interval"
                        continue
                    fi
                    armed=1
                    echo -e "${YELLOW}Health watchdog: startup grace expired without a successful probe for ${url}; counting failures now.${NC}"
                fi
                failures=$((failures + 1))
                echo -e "${YELLOW}Health watchdog: local probe failed (${failures}/${max_failures}) for ${url}; error=${HEALTH_PROBE_ERROR:-unknown}.${NC}"
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
