#!/bin/bash
# AgentArk Installer
# Think. Act. Remember. Securely.
#
# Usage: curl -sSL https://raw.githubusercontent.com/agentark-ai/AgentArk/main/scripts/install.sh | bash

set -e

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
BOLD='\033[1m'
NC='\033[0m'

INSTALL_DIR="$HOME/agentark"
SOURCE_DIR="$INSTALL_DIR/source"
REPO_URL="https://github.com/agentark-ai/AgentArk.git"

echo ""
echo -e "${BOLD}=========================================${NC}"
echo -e "${BOLD}  AgentArk Installer${NC}"
echo -e "  Think. Act. Remember. Securely."
echo -e "${BOLD}=========================================${NC}"
echo ""

install_docker() {
    echo -e "${YELLOW}Docker not found. Installing...${NC}"
    if [ -f /etc/os-release ]; then
        . /etc/os-release
        case "$ID" in
            ubuntu|debian|pop|linuxmint|elementary)
                echo -e "${CYAN}Detected: $PRETTY_NAME${NC}"
                sudo apt-get update -qq
                sudo apt-get install -y -qq ca-certificates curl gnupg
                sudo install -m 0755 -d /etc/apt/keyrings
                curl -fsSL https://download.docker.com/linux/$ID/gpg | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg 2>/dev/null
                sudo chmod a+r /etc/apt/keyrings/docker.gpg
                echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/$ID $(. /etc/os-release && echo \"$VERSION_CODENAME\") stable" | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
                sudo apt-get update -qq
                sudo apt-get install -y -qq docker-ce docker-ce-cli containerd.io docker-compose-plugin
                ;;
            fedora)
                echo -e "${CYAN}Detected: $PRETTY_NAME${NC}"
                sudo dnf -y install dnf-plugins-core
                sudo dnf config-manager --add-repo https://download.docker.com/linux/fedora/docker-ce.repo
                sudo dnf install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
                ;;
            centos|rhel|rocky|almalinux)
                echo -e "${CYAN}Detected: $PRETTY_NAME${NC}"
                sudo yum install -y yum-utils
                sudo yum-config-manager --add-repo https://download.docker.com/linux/centos/docker-ce.repo
                sudo yum install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
                ;;
            arch|manjaro)
                echo -e "${CYAN}Detected: $PRETTY_NAME${NC}"
                sudo pacman -Sy --noconfirm docker docker-compose
                ;;
            *)
                echo -e "${RED}Unsupported distro: $ID${NC}"
                echo -e "Please install Docker manually: ${CYAN}https://docs.docker.com/engine/install/${NC}"
                exit 1
                ;;
        esac
    elif [ "$(uname)" = "Darwin" ]; then
        echo -e "${RED}macOS detected.${NC}"
        echo -e "Please install Docker Desktop: ${CYAN}https://docs.docker.com/desktop/install/mac-install/${NC}"
        exit 1
    else
        echo -e "${RED}Unsupported OS.${NC}"
        echo -e "Please install Docker manually: ${CYAN}https://docs.docker.com/engine/install/${NC}"
        exit 1
    fi

    sudo systemctl start docker 2>/dev/null || true
    sudo systemctl enable docker 2>/dev/null || true

    if ! groups | grep -q docker; then
        sudo usermod -aG docker "$USER"
        echo -e "${YELLOW}Added $USER to docker group. You may need to log out and back in.${NC}"
    fi

    echo -e "${GREEN}Docker installed successfully.${NC}"
}

if command -v docker &>/dev/null; then
    echo -e "${GREEN}[1/4] Docker found.${NC}"
else
    install_docker
    echo -e "${GREEN}[1/4] Docker installed.${NC}"
fi

if docker compose version &>/dev/null; then
    COMPOSE="docker compose"
elif command -v docker-compose &>/dev/null; then
    COMPOSE="docker-compose"
else
    echo -e "${RED}docker compose not found. Please install Docker Compose v2.${NC}"
    exit 1
fi
echo -e "${GREEN}[2/4] Docker Compose found.${NC}"

mkdir -p "$INSTALL_DIR"

if [ ! -d "$SOURCE_DIR/.git" ]; then
    echo -e "${CYAN}Cloning AgentArk source into $SOURCE_DIR...${NC}"
    docker run --rm -v "$INSTALL_DIR:/work" -w /work alpine/git clone --depth 1 "$REPO_URL" source
else
    echo -e "${GREEN}Existing source checkout found at $SOURCE_DIR${NC}"
fi

cat > "$INSTALL_DIR/docker-compose.yml" <<COMPOSE_EOF
services:
  agentark:
    build: "$SOURCE_DIR"
    image: agentark:latest
    container_name: agentark
    restart: unless-stopped
    ports:
      - "8990:8990"
    volumes:
      - agentark-data:/app/data
      - agentark-config:/app/config
      - "${SOURCE_DIR}:/workspace/agentark"
      - "${INSTALL_DIR}:${INSTALL_DIR}"
    depends_on:
      - docker-socket-proxy
    environment:
      - RUST_LOG=info,sqlx::query=warn,sea_orm=warn,hyper=warn,reqwest=warn
      - AGENTARK_CONFIG=/app/config
      - AGENTARK_DATA=/app/data
      - AGENTARK_BIND=0.0.0.0:8990
      - DOCKER_HOST=tcp://docker-socket-proxy:2375
      - AGENTARK_DEBUG=\${AGENTARK_DEBUG:-false}
      - AGENTARK_TUNNEL=\${AGENTARK_TUNNEL:-false}
      - AGENTARK_MASTER_PASSWORD=\${AGENTARK_MASTER_PASSWORD:-}
      - TUNNEL_TOKEN=\${TUNNEL_TOKEN:-}
    networks:
      - agent-network
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8990/health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 5s
    deploy:
      resources:
        limits:
          cpus: '2'
          memory: 2G

  docker-socket-proxy:
    image: tecnativa/docker-socket-proxy:latest
    container_name: agentark-docker-proxy
    restart: unless-stopped
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
    environment:
      - CONTAINERS=1
      - IMAGES=1
      - POST=1
      - EXEC=0
      - VOLUMES=0
      - NETWORKS=0
      - SWARM=0
      - SECRETS=0
      - NODES=0
      - SERVICES=0
      - TASKS=0
      - BUILD=0
      - COMMIT=0
      - CONFIGS=0
      - DISTRIBUTION=0
      - PLUGINS=0
      - SYSTEM=0
    networks:
      - agent-network
    deploy:
      resources:
        limits:
          cpus: '0.5'
          memory: 128M

volumes:
  agentark-data:
    name: agentark-data
  agentark-config:
    name: agentark-config

networks:
  agent-network:
    driver: bridge
COMPOSE_EOF

echo -e "${GREEN}[3/4] Configuration created at $INSTALL_DIR${NC}"

cat > "$INSTALL_DIR/agentark.sh" << 'SCRIPT_EOF'
#!/bin/bash
set -e
cd "$(dirname "$0")"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

case "${1:-start}" in
    start)
        echo -e "${GREEN}Starting AgentArk...${NC}"
        docker compose up -d --build
        echo ""
        echo -e "${GREEN}AgentArk is running!${NC}"
        echo -e "  Web UI:  ${CYAN}http://localhost:8990${NC}"
        ;;
    tunnel)
        echo -e "${GREEN}Starting AgentArk with remote access...${NC}"
        AGENTARK_TUNNEL=true docker compose up -d --build
        echo ""
        echo -e "  Local:   ${CYAN}http://localhost:8990${NC}"
        echo -e "  Remote:  ${CYAN}Your Cloudflare URL will appear in the Web UI${NC}"
        ;;
    stop)
        echo -e "${YELLOW}Stopping AgentArk...${NC}"
        docker compose down
        echo -e "${GREEN}Stopped. Your data is preserved.${NC}"
        ;;
    restart)
        docker compose up -d agentark
        ;;
    update)
        echo -e "${YELLOW}Updating AgentArk source and rebuilding...${NC}"
        docker run --rm -v "$(pwd):/work" -w /work alpine/git git -C /work/source pull --ff-only || true
        docker compose build agentark
        docker compose up -d agentark
        echo -e "${GREEN}Updated! Your data is intact.${NC}"
        ;;
    logs)
        docker compose logs -f
        ;;
    status)
        docker compose ps
        ;;
    uninstall)
        echo -e "${YELLOW}This will stop AgentArk and remove containers.${NC}"
        echo -e "${BOLD}Your data volumes and source checkout will be preserved.${NC}"
        read -p "Continue? [y/N] " confirm
        if [ "$confirm" = "y" ] || [ "$confirm" = "Y" ]; then
            docker compose down
            echo -e "${GREEN}Removed. Data volumes kept. Source remains in ./source.${NC}"
        fi
        ;;
    *)
        echo "Usage: ./agentark.sh [start|tunnel|stop|restart|update|logs|status|uninstall]"
        ;;
esac
SCRIPT_EOF

chmod +x "$INSTALL_DIR/agentark.sh"

echo -e "${CYAN}Building AgentArk image from local source (this may take a few minutes)...${NC}"
cd "$INSTALL_DIR"

echo -e "${GREEN}[4/4] Starting AgentArk...${NC}"
$COMPOSE up -d --build

echo ""
echo -e "${BOLD}=========================================${NC}"
echo -e "${GREEN}  AgentArk is running!${NC}"
echo -e "${BOLD}=========================================${NC}"
echo ""
echo -e "  Web UI:  ${CYAN}http://localhost:8990${NC}"
echo ""
echo -e "  Manage:  ${BOLD}cd $INSTALL_DIR && ./agentark.sh${NC}"
echo ""
echo -e "  Commands:"
echo -e "    ./agentark.sh start     Start AgentArk"
echo -e "    ./agentark.sh tunnel    Start with remote access"
echo -e "    ./agentark.sh stop      Stop AgentArk"
echo -e "    ./agentark.sh update    Pull source and rebuild"
echo -e "    ./agentark.sh logs      View logs"
echo -e "    ./agentark.sh status    Show status"
echo ""
echo -e "${YELLOW}Writable source checkout: $SOURCE_DIR${NC}"
echo -e "${YELLOW}Data is stored in Docker volumes and survives rebuilds.${NC}"
echo ""
