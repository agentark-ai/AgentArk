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

if [ ! -f "$SOURCE_DIR/docker-compose.yml" ]; then
    echo -e "${RED}Missing $SOURCE_DIR/docker-compose.yml after clone.${NC}"
    exit 1
fi

echo -e "${GREEN}[3/4] Source checkout ready at $SOURCE_DIR${NC}"

cat > "$INSTALL_DIR/agentark" << 'SCRIPT_EOF'
#!/bin/bash
# AgentArk CLI — simple commands for your AI agent
# Usage: agentark chat | pulse | start | stop | logs | status | update

set -e

# Find install dir (resolve symlinks)
SCRIPT_PATH="$(readlink -f "$0" 2>/dev/null || realpath "$0" 2>/dev/null || echo "$0")"
AGENTARK_DIR="$(cd "$(dirname "$SCRIPT_PATH")" && pwd)"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

compose() {
    docker compose -f "$AGENTARK_DIR/source/docker-compose.yml" "$@"
}

case "${1:-help}" in
    chat)
        docker exec -it agentark-control /app/agentark --chat
        ;;
    pulse)
        echo -e "${CYAN}Running ArkPulse health check...${NC}"
        docker exec agentark-control /app/agentark --pulse
        ;;
    start)
        echo -e "${GREEN}Starting AgentArk...${NC}"
        compose up -d
        echo ""
        echo -e "${GREEN}AgentArk is running!${NC}"
        echo -e "  Web UI:  ${CYAN}http://localhost:8990${NC}"
        ;;
    tunnel)
        echo -e "${GREEN}Starting AgentArk with remote access...${NC}"
        AGENTARK_TUNNEL=true compose up -d
        echo ""
        echo -e "  Local:   ${CYAN}http://localhost:8990${NC}"
        echo -e "  Remote:  ${CYAN}Your Cloudflare URL will appear in the Web UI${NC}"
        ;;
    stop)
        echo -e "${YELLOW}Stopping AgentArk...${NC}"
        compose down
        echo -e "${GREEN}Stopped. Your data is preserved.${NC}"
        echo -e "${YELLOW}Run 'docker compose down -v' to reset Postgres and all volumes.${NC}"
        ;;
    restart)
        compose down && compose up -d
        ;;
    update)
        echo -e "${YELLOW}Updating AgentArk compose files and pulling the latest image...${NC}"
        docker run --rm -v "$AGENTARK_DIR:/work" -w /work alpine/git git -C /work/source pull --ff-only || true
        compose pull
        compose up -d
        echo -e "${GREEN}Updated! Your data is intact.${NC}"
        ;;
    logs)
        compose logs -f --tail=100
        ;;
    status)
        compose ps
        ;;
    setup)
        docker exec -it agentark-control /app/agentark --setup
        ;;
    uninstall)
        echo -e "${YELLOW}This will stop AgentArk and remove containers.${NC}"
        echo -e "${BOLD}Your data volumes and source checkout will be preserved.${NC}"
        read -p "Continue? [y/N] " confirm
        if [ "$confirm" = "y" ] || [ "$confirm" = "Y" ]; then
            compose down
            rm -f /usr/local/bin/agentark 2>/dev/null || true
            echo -e "${GREEN}Removed. Data volumes kept. Source remains in $AGENTARK_DIR/source.${NC}"
        fi
        ;;
    *)
        echo "AgentArk CLI"
        echo ""
        echo "Usage: agentark <command>"
        echo ""
        echo "  chat       Interactive CLI chat with your agent"
        echo "  pulse      Run ArkPulse health check"
        echo "  start      Start AgentArk (or 'tunnel' for remote access)"
        echo "  stop       Stop AgentArk"
        echo "  restart    Restart AgentArk"
        echo "  logs       View live logs"
        echo "  status     Show running containers"
        echo "  update     Pull latest image and restart (preserves data)"
        echo "  setup      Run setup wizard"
        echo "  uninstall  Stop and remove containers"
        ;;
esac
SCRIPT_EOF

chmod +x "$INSTALL_DIR/agentark"

# Install global 'agentark' command on PATH
if [ -d /usr/local/bin ] && [ -w /usr/local/bin ]; then
    ln -sf "$INSTALL_DIR/agentark" /usr/local/bin/agentark
    echo -e "${GREEN}Installed 'agentark' command globally.${NC}"
elif command -v sudo &>/dev/null; then
    sudo ln -sf "$INSTALL_DIR/agentark" /usr/local/bin/agentark
    echo -e "${GREEN}Installed 'agentark' command globally.${NC}"
else
    echo -e "${YELLOW}Could not install to /usr/local/bin. Add $INSTALL_DIR to your PATH:${NC}"
    echo -e "  export PATH=\"$INSTALL_DIR:\$PATH\""
fi

echo -e "${CYAN}Downloading AgentArk container image (first run may take a few minutes)...${NC}"
cd "$SOURCE_DIR"

port_in_use() {
    local port="$1"
    if command -v ss &>/dev/null; then
        ss -ltn "( sport = :$port )" 2>/dev/null | tail -n +2 | grep -q .
        return $?
    fi
    if command -v lsof &>/dev/null; then
        lsof -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1
        return $?
    fi
    return 1
}

warn_if_port_in_use() {
    local port="$1"
    local service="$2"
    if port_in_use "$port"; then
        echo -e "${YELLOW}Warning: TCP port ${port} is already in use. ${service} may fail to start unless you stop the existing listener or override the port.${NC}"
    fi
}

POSTGRES_PORT="${AGENTARK_POSTGRES_PORT:-5432}"
warn_if_port_in_use "$POSTGRES_PORT" "Postgres"
warn_if_port_in_use "8990" "AgentArk Web UI"

echo -e "${GREEN}[4/4] Starting AgentArk...${NC}"
$COMPOSE pull
$COMPOSE up -d

echo ""
echo -e "${BOLD}=========================================${NC}"
echo -e "${GREEN}  AgentArk is running!${NC}"
echo -e "${BOLD}=========================================${NC}"
echo ""
echo -e "  Web UI:  ${CYAN}http://localhost:8990${NC}"
echo ""
echo -e "  Commands (run from anywhere):"
echo -e "    ${BOLD}agentark chat${NC}       Interactive CLI chat"
echo -e "    ${BOLD}agentark pulse${NC}      Run ArkPulse health check"
echo -e "    ${BOLD}agentark stop${NC}       Stop AgentArk"
echo -e "    ${BOLD}agentark update${NC}     Pull latest image and restart"
echo -e "    ${BOLD}agentark logs${NC}       View logs"
echo -e "    ${BOLD}agentark status${NC}     Show status"
echo ""
echo -e "${YELLOW}Compose checkout: $SOURCE_DIR${NC}"
echo -e "${YELLOW}App data is stored in Docker volumes and survives updates.${NC}"
echo -e "${YELLOW}Postgres lives in its own volume; use 'docker compose down -v' to reset everything.${NC}"
echo ""
