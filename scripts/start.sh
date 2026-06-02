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
#   ./scripts/start.sh backup       - Backup Docker volumes
#   ./scripts/start.sh status       - Show running containers

set -e

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
BOLD='\033[1m'
NC='\033[0m'

compose() {
    docker compose "$@"
}

compose_dev() {
    docker compose -f docker-compose.yml -f docker-compose.dev.yml "$@"
}

pull_runtime_images() {
    compose pull postgres agentark-control agentark-embeddings agentark-executor agentark-workspace
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
    (verify_lightpanda_runtime >/dev/null 2>&1 || true) &
}

verify_gepa_optimizer_runtime() {
    local attempts=20

    echo -e "${CYAN}Verifying bundled GEPA optimizer runtime...${NC}"
    while [ "$attempts" -gt 0 ]; do
        if compose exec -T agentark-control sh -lc '/opt/agentark-gepa/bin/python -c "import dspy" >/dev/null 2>&1' >/dev/null 2>&1; then
            echo -e "${GREEN}GEPA optimizer is available inside the AgentArk runtime.${NC}"
            return 0
        fi
        attempts=$((attempts - 1))
        sleep 2
    done

    echo -e "${RED}GEPA optimizer is missing from the bundled AgentArk runtime. Update or rebuild before running ArkEvolve GEPA.${NC}"
    return 1
}

verify_gepa_optimizer_runtime_async() {
    (verify_gepa_optimizer_runtime >/dev/null 2>&1 || true) &
}

backup_volume() {
    local volume="$1"
    local archive="$2"
    local backup_dir="$3"

    echo -e "  ${CYAN}${volume}${NC} -> ${archive}"
    docker run --rm \
        -v "${volume}:/data:ro" \
        -v "$(pwd)/${backup_dir}:/backup" \
        alpine tar czf "/backup/${archive}" -C /data .
}

case "${1:-start}" in
    start)
        echo -e "${GREEN}Starting AgentArk...${NC}"
        compose up -d
        verify_lightpanda_runtime_async
        verify_gepa_optimizer_runtime_async
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

            echo -e "${YELLOW}Permanent tunnel tokens are stored inside AgentArk settings.${NC}"
            echo -e "Open ${BOLD}Web UI -> Settings -> Remote Access${NC} and paste the token there."
            echo ""
        fi

        echo -e "${GREEN}Starting AgentArk with remote access...${NC}"
        (export AGENTARK_TUNNEL=true; compose up -d)
        verify_lightpanda_runtime_async
        verify_gepa_optimizer_runtime_async
        echo ""
        echo -e "${GREEN}AgentArk is starting with secure tunnel!${NC}"
        echo ""
        echo -e "  Local:   ${CYAN}http://localhost:8990${NC}"
        echo -e "  Remote:  ${CYAN}Your Cloudflare URL will appear in the Web UI${NC}"
        echo ""

        echo -e "  Using:   ${CYAN}Remote access settings from AgentArk${NC}"
        echo -e "  ${YELLOW}For a permanent URL, configure Settings -> Remote Access in the Web UI.${NC}"
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
        verify_gepa_optimizer_runtime_async
        ;;
    logs)
        compose logs -f
        ;;
    update)
        echo -e "${YELLOW}Updating AgentArk (your data will be preserved)...${NC}"
        pull_runtime_images
        compose up -d --build
        verify_lightpanda_runtime_async
        verify_gepa_optimizer_runtime_async
        echo -e "${GREEN}Update complete! Your data is intact.${NC}"
        ;;
    build)
        echo -e "${YELLOW}Building AgentArk from this checkout and force-recreating containers (your data will be preserved)...${NC}"
        (export AGENTARK_IMAGE="${AGENTARK_IMAGE:-agentark:dev}"; compose_dev up -d --build --force-recreate)
        verify_lightpanda_runtime_async
        verify_gepa_optimizer_runtime_async
        echo -e "${GREEN}Local build complete! Your data is intact.${NC}"
        ;;
    backup)
        BACKUP_DIR="./backups/$(date +%Y%m%d_%H%M%S)"
        mkdir -p "$BACKUP_DIR"
        echo -e "${YELLOW}Backing up AgentArk volumes to $BACKUP_DIR...${NC}"
        backup_volume agentark-data agentark-data.tar.gz "$BACKUP_DIR"
        backup_volume agentark-config agentark-config.tar.gz "$BACKUP_DIR"
        backup_volume agentark-postgres-data agentark-postgres-data.tar.gz "$BACKUP_DIR"
        backup_volume agentark-secrets agentark-secrets.tar.gz "$BACKUP_DIR"
        echo -e "${GREEN}Backup complete!${NC}"
        echo -e "${YELLOW}Keep agentark-secrets.tar.gz with the Postgres/config backups; it is required to unlock install-managed encrypted data.${NC}"
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
        echo "  backup         Backup Docker volumes"
        echo "  status         Show running containers"
        echo "  chat           Interactive CLI chat with the agent"
        echo "  pulse          Run ArkPulse health check"
        exit 1
        ;;
esac
