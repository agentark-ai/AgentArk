#!/bin/bash
# AgentArk Easy Start Script
#
# This script ensures your data is always preserved across updates.
# Just run: ./scripts/start.sh
#
# Commands:
#   ./scripts/start.sh              - Start AgentArk (local access only)
#   ./scripts/start.sh tunnel       - Start with remote access (auto-starts Cloudflare tunnel)
#   ./scripts/start.sh tunnel setup - Set up a permanent custom domain (free Cloudflare account)
#   ./scripts/start.sh stop         - Stop AgentArk
#   ./scripts/start.sh restart      - Restart AgentArk
#   ./scripts/start.sh logs         - View logs
#   ./scripts/start.sh update       - Pull latest image and restart (preserves data)
#   ./scripts/start.sh build        - Build from this checkout and restart
#   ./scripts/start.sh backup       - Backup your data
#   ./scripts/start.sh status       - Show running containers

set -e

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
BOLD='\033[1m'
NC='\033[0m'
AGENTARK_LOCAL_ENV="${AGENTARK_LOCAL_ENV:-.agentark/local.env}"

compose() {
    docker compose --env-file "$AGENTARK_LOCAL_ENV" "$@"
}

compose_dev() {
    docker compose --env-file "$AGENTARK_LOCAL_ENV" -f docker-compose.yml -f docker-compose.dev.yml "$@"
}

read_env_value() {
    local key="$1"
    local file="$2"
    [ -f "$file" ] || return 0
    awk -F= -v key="$key" '$1 == key { sub(/^[^=]*=/, ""); print; exit }' "$file"
}

upsert_managed_env_value() {
    local key="$1"
    local value="$2"
    mkdir -p "$(dirname "$AGENTARK_LOCAL_ENV")"
    if [ -f "$AGENTARK_LOCAL_ENV" ] && grep -q "^${key}=" "$AGENTARK_LOCAL_ENV"; then
        local escaped
        escaped="$(printf '%s' "$value" | sed 's/[&|]/\\&/g')"
        sed -i.bak "s|^${key}=.*|${key}=${escaped}|" "$AGENTARK_LOCAL_ENV"
        rm -f "${AGENTARK_LOCAL_ENV}.bak"
    else
        printf '%s=%s\n' "$key" "$value" >> "$AGENTARK_LOCAL_ENV"
    fi
}

ensure_postgres_password() {
    mkdir -p "$(dirname "$AGENTARK_LOCAL_ENV")"
    touch "$AGENTARK_LOCAL_ENV"
    if grep -q '^AGENTARK_POSTGRES_PASSWORD=' "$AGENTARK_LOCAL_ENV"; then
        local tmp_file
        tmp_file="$(mktemp)"
        grep -v '^AGENTARK_POSTGRES_PASSWORD=' "$AGENTARK_LOCAL_ENV" > "$tmp_file"
        cat "$tmp_file" > "$AGENTARK_LOCAL_ENV"
        rm -f "$tmp_file"
    fi
    unset AGENTARK_POSTGRES_PASSWORD
    echo -e "${GREEN}Local Postgres password is managed inside the Docker volume agentark-secrets.${NC}"
}

verify_lightpanda_runtime() {
    local attempts=20

    echo -e "${CYAN}Verifying bundled Lightpanda runtime...${NC}"
    while [ "$attempts" -gt 0 ]; do
        if compose exec -T agentark-control sh -lc 'command -v lightpanda >/dev/null 2>&1' >/dev/null 2>&1; then
            echo -e "${GREEN}Lightpanda is available inside the AgentArk runtime.${NC}"
            return 0
        fi
        attempts=$((attempts - 1))
        sleep 2
    done

    echo -e "${RED}Lightpanda is missing from the bundled AgentArk runtime. Update or rebuild before relying on the free search fallback.${NC}"
    return 1
}

verify_lightpanda_runtime_async() {
    mkdir -p "$(dirname "$AGENTARK_LOCAL_ENV")"
    (verify_lightpanda_runtime > "$(dirname "$AGENTARK_LOCAL_ENV")/lightpanda-check.log" 2>&1 || true) &
}

case "${1:-start}" in
    start)
        ensure_postgres_password || exit 1
        echo -e "${GREEN}Starting AgentArk...${NC}"
        compose up -d
        verify_lightpanda_runtime_async
        echo ""
        echo -e "${GREEN}AgentArk is running!${NC}"
        echo -e "  Web UI:  ${CYAN}http://localhost:8990${NC}"
        echo ""
        echo -e "${YELLOW}Your data is safely stored in Docker volumes (agentark-data, agentark-config)${NC}"
        echo ""
        echo -e "Want to access from anywhere? Enable the tunnel from the Web UI"
        echo -e "  or run: ${BOLD}./scripts/start.sh tunnel${NC}"
        ;;
    tunnel)
        if [ "${2}" = "setup" ]; then
            # Named tunnel setup (permanent custom domain)
            echo ""
            echo -e "${BOLD}━━━ Permanent Custom Domain Setup ━━━${NC}"
            echo ""
            echo -e "This gives you a ${CYAN}permanent URL${NC} like ${CYAN}agent.yourdomain.com${NC}"
            echo -e "instead of a random URL that changes on restart."
            echo ""
            echo -e "${BOLD}Setup (5 minutes, free):${NC}"
            echo ""
            echo -e "  1. Go to ${CYAN}https://one.dash.cloudflare.com${NC}"
            echo -e "  2. Sign up / log in (free plan works)"
            echo -e "  3. Go to: ${BOLD}Networks → Tunnels → Create a tunnel${NC}"
            echo -e "  4. Name it ${CYAN}agentark${NC}"
            echo -e "  5. Copy the tunnel token"
            echo -e "  6. Add a public hostname pointing to: ${CYAN}http://localhost:8990${NC}"
            echo ""
            read -p "Paste your Tunnel Token here (or press Enter to cancel): " token
            echo ""

            if [ -z "$token" ]; then
                echo -e "${YELLOW}Cancelled. You can run this again anytime.${NC}"
                exit 0
            fi

            upsert_managed_env_value TUNNEL_TOKEN "$token"
            echo -e "${GREEN}Token saved to ${AGENTARK_LOCAL_ENV}${NC}"
            echo ""
        fi

        ensure_postgres_password || exit 1
        echo -e "${GREEN}Starting AgentArk with remote access...${NC}"
        AGENTARK_TUNNEL=true compose up -d
        verify_lightpanda_runtime_async
        echo ""
        echo -e "${GREEN}AgentArk is starting with secure tunnel!${NC}"
        echo ""
        echo -e "  Local:   ${CYAN}http://localhost:8990${NC}"
        echo -e "  Remote:  ${CYAN}Your Cloudflare URL will appear in the Web UI${NC}"
        echo ""

        # Check if named or quick tunnel
        if [ -f "$AGENTARK_LOCAL_ENV" ]; then
            set -a
            . "$AGENTARK_LOCAL_ENV" 2>/dev/null || true
            set +a
        fi
        if [ -n "$TUNNEL_TOKEN" ]; then
            echo -e "  Using:   ${CYAN}Permanent custom domain (configured in Cloudflare)${NC}"
        else
            echo -e "  Using:   ${CYAN}Quick tunnel (random URL, changes on restart)${NC}"
            echo -e "  ${YELLOW}For a permanent URL, run:${NC}"
            echo -e "    ${BOLD}./scripts/start.sh tunnel setup${NC}"
        fi
        echo ""
        echo -e "  Manage the tunnel from: ${BOLD}Web UI → Settings → Remote Access${NC}"
        echo ""
        echo -e "${YELLOW}All traffic is encrypted. API key protects all endpoints.${NC}"
        ;;
    stop)
        echo -e "${YELLOW}Stopping AgentArk...${NC}"
        compose down
        echo -e "${GREEN}AgentArk stopped. Your data is preserved.${NC}"
        ;;
    restart)
        echo -e "${YELLOW}Restarting AgentArk...${NC}"
        compose restart agentark-control agentark-workspace agentark-executor agentark-embeddings
        verify_lightpanda_runtime_async
        ;;
    logs)
        compose logs -f
        ;;
    update)
        ensure_postgres_password || exit 1
        echo -e "${YELLOW}Updating AgentArk (your data will be preserved)...${NC}"
        compose pull
        compose up -d
        verify_lightpanda_runtime_async
        echo -e "${GREEN}Update complete! Your data is intact.${NC}"
        ;;
    build)
        ensure_postgres_password || exit 1
        echo -e "${YELLOW}Building AgentArk from this checkout and force-recreating containers (your data will be preserved)...${NC}"
        AGENTARK_IMAGE=${AGENTARK_IMAGE:-agentark:dev} compose_dev up -d --build --force-recreate
        verify_lightpanda_runtime_async
        echo -e "${GREEN}Local build complete! Your data is intact.${NC}"
        ;;
    backup)
        BACKUP_DIR="./backups/$(date +%Y%m%d_%H%M%S)"
        mkdir -p "$BACKUP_DIR"
        echo -e "${YELLOW}Backing up data to $BACKUP_DIR...${NC}"
        docker run --rm -v agentark-data:/data -v "$(pwd)/$BACKUP_DIR":/backup alpine tar czf /backup/agentark-data.tar.gz -C /data .
        docker run --rm -v agentark-config:/data -v "$(pwd)/$BACKUP_DIR":/backup alpine tar czf /backup/agentark-config.tar.gz -C /data .
        echo -e "${GREEN}Backup complete!${NC}"
        ;;
    status)
        echo -e "${BOLD}AgentArk Status:${NC}"
        compose ps
        ;;
    chat)
        docker exec -it agentark-control /app/agentark --chat
        ;;
    pulse)
        echo -e "${CYAN}Running ArkPulse health check...${NC}"
        docker exec -it agentark-control /app/agentark --chat <<< "run arkpulse now"
        ;;
    *)
        echo "Usage: ./scripts/start.sh [start|tunnel|stop|restart|logs|update|build|backup|status|chat|pulse]"
        echo ""
        echo "  start          Start AgentArk (local access only)"
        echo "  tunnel         Start with remote access (auto-starts Cloudflare tunnel)"
        echo "  tunnel setup   Set up permanent custom domain (free Cloudflare account)"
        echo "  stop           Stop AgentArk"
        echo "  restart        Restart AgentArk"
        echo "  logs           View logs"
        echo "  update         Pull latest image and restart (preserves data)"
        echo "  build          Build from this checkout and restart"
        echo "  backup         Backup your data"
        echo "  status         Show running containers"
        echo "  chat           Interactive CLI chat with the agent"
        echo "  pulse          Run ArkPulse health check"
        exit 1
        ;;
esac
