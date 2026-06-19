#!/bin/bash
# AgentArk Docker installer.
#
# Usage:
#   curl -sSL https://raw.githubusercontent.com/agentark-ai/AgentArk/main/scripts/install.sh | bash

set -euo pipefail

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
BOLD='\033[1m'
NC='\033[0m'

INSTALL_DIR="${HOME}/agentark"
RUNTIME_DIR="${INSTALL_DIR}/runtime"
RELEASE_REPO="${AGENTARK_RELEASE_REPO:-agentark-ai/AgentArk}"
IMAGE_REPOSITORY="${AGENTARK_IMAGE_REPOSITORY:-ghcr.io/agentark-ai/agentark}"
RUNTIME_REF="${AGENTARK_RUNTIME_REF:-main}"

truthy() {
    case "${1:-}" in
        1|true|TRUE|yes|YES|y|Y|on|ON) return 0 ;;
        *) return 1 ;;
    esac
}

confirm() {
    local prompt="$1"
    local answer=""

    if truthy "${AGENTARK_ASSUME_YES:-}"; then
        return 0
    fi

    read -r -p "${prompt}" answer
    case "${answer}" in
        y|Y|yes|YES|Yes) return 0 ;;
        *) return 1 ;;
    esac
}

version_from_tag() {
    printf '%s' "${1#v}"
}

latest_release_tag() {
    curl -fsSL "https://api.github.com/repos/${RELEASE_REPO}/releases/latest" \
        | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
        | head -n 1
}

docker_ready() {
    command -v docker >/dev/null 2>&1 \
        && docker info >/dev/null 2>&1 \
        && docker compose version >/dev/null 2>&1
}

wait_for_docker() {
    local attempts=90
    while [ "${attempts}" -gt 0 ]; do
        if docker_ready; then
            return 0
        fi
        attempts=$((attempts - 1))
        sleep 2
    done
    return 1
}

install_docker_linux() {
    if ! confirm "Docker is required. Install Docker now? [y/N] "; then
        echo -e "${RED}Docker is required.${NC}"
        echo -e "Install Docker: ${CYAN}https://docs.docker.com/engine/install/${NC}"
        exit 1
    fi

    . /etc/os-release
    case "${ID}" in
        ubuntu|debian|pop|linuxmint|elementary)
            sudo apt-get update -qq
            sudo apt-get install -y -qq ca-certificates curl gnupg
            sudo install -m 0755 -d /etc/apt/keyrings
            curl -fsSL "https://download.docker.com/linux/${ID}/gpg" | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg 2>/dev/null
            sudo chmod a+r /etc/apt/keyrings/docker.gpg
            echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/${ID} ${VERSION_CODENAME} stable" \
                | sudo tee /etc/apt/sources.list.d/docker.list >/dev/null
            sudo apt-get update -qq
            sudo apt-get install -y -qq docker-ce docker-ce-cli containerd.io docker-compose-plugin
            ;;
        fedora)
            sudo dnf -y install dnf-plugins-core
            sudo dnf config-manager --add-repo https://download.docker.com/linux/fedora/docker-ce.repo
            sudo dnf install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
            ;;
        centos|rhel|rocky|almalinux)
            sudo yum install -y yum-utils
            sudo yum-config-manager --add-repo https://download.docker.com/linux/centos/docker-ce.repo
            sudo yum install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
            ;;
        arch|manjaro)
            sudo pacman -Sy --noconfirm docker docker-compose
            ;;
        *)
            echo -e "${RED}Unsupported distro: ${ID}${NC}"
            echo -e "Install Docker manually: ${CYAN}https://docs.docker.com/engine/install/${NC}"
            exit 1
            ;;
    esac

    sudo systemctl start docker 2>/dev/null || true
    sudo systemctl enable docker 2>/dev/null || true

    if ! groups | grep -q docker; then
        sudo usermod -aG docker "$USER" 2>/dev/null || true
    fi
}

install_docker_macos() {
    if ! confirm "Docker Desktop is required. Install it now? [y/N] "; then
        echo -e "${RED}Docker Desktop is required.${NC}"
        echo -e "Install Docker Desktop: ${CYAN}https://docs.docker.com/desktop/install/mac-install/${NC}"
        exit 1
    fi

    if command -v brew >/dev/null 2>&1; then
        brew install --cask docker
    else
        echo -e "${RED}Homebrew was not found.${NC}"
        echo -e "Install Docker Desktop manually: ${CYAN}https://docs.docker.com/desktop/install/mac-install/${NC}"
        exit 1
    fi
}

ensure_docker() {
    if docker_ready; then
        return 0
    fi

    if ! command -v docker >/dev/null 2>&1; then
        if [ "$(uname)" = "Darwin" ]; then
            install_docker_macos
        elif [ -f /etc/os-release ]; then
            install_docker_linux
        else
            echo -e "${RED}Docker is required.${NC}"
            echo -e "Install Docker: ${CYAN}https://docs.docker.com/get-docker/${NC}"
            exit 1
        fi
    fi

    if [ "$(uname)" = "Darwin" ]; then
        echo -e "${CYAN}Starting Docker Desktop...${NC}"
        open -a Docker >/dev/null 2>&1 || true
    else
        sudo systemctl start docker 2>/dev/null || true
    fi

    if ! wait_for_docker; then
        if [ "$(uname)" != "Darwin" ] && ! groups | grep -q docker; then
            echo -e "${YELLOW}Docker was installed, but this shell is not in the docker group yet.${NC}"
            echo -e "Log out and back in, then rerun this installer."
            exit 1
        fi
        echo -e "${RED}Docker did not become ready.${NC}"
        echo -e "Open Docker Desktop or start the Docker service, then rerun this installer."
        exit 1
    fi
}

port_in_use() {
    local port="$1"
    if command -v ss >/dev/null 2>&1; then
        ss -ltn "( sport = :${port} )" 2>/dev/null | tail -n +2 | grep -q .
        return $?
    fi
    if command -v lsof >/dev/null 2>&1; then
        lsof -iTCP:"${port}" -sTCP:LISTEN >/dev/null 2>&1
        return $?
    fi
    return 1
}

warn_if_port_in_use() {
    local port="$1"
    local service="$2"
    if port_in_use "${port}"; then
        echo -e "${YELLOW}Warning: TCP port ${port} is already in use. ${service} may fail to start.${NC}"
    fi
}

download_runtime_files() {
    local raw_base="https://raw.githubusercontent.com/${RELEASE_REPO}/${RUNTIME_REF}"

    mkdir -p "${RUNTIME_DIR}/scripts"
    curl -fsSL "${raw_base}/docker-compose.yml" -o "${RUNTIME_DIR}/docker-compose.yml"
    curl -fsSL "${raw_base}/scripts/start.sh" -o "${RUNTIME_DIR}/scripts/start.sh"
    chmod +x "${RUNTIME_DIR}/scripts/start.sh"
}

write_agentark_command() {
    cat > "${INSTALL_DIR}/agentark" << SCRIPT_EOF
#!/bin/bash
set -e
AGENTARK_DIR="\$(cd "\$(dirname "\$0")" && pwd)"
export AGENTARK_RELEASE_REPO="\${AGENTARK_RELEASE_REPO:-${RELEASE_REPO}}"
export AGENTARK_RELEASE_TAG="\${AGENTARK_RELEASE_TAG:-${TARGET_RELEASE_TAG}}"
export AGENTARK_IMAGE="\${AGENTARK_IMAGE:-${IMAGE_REPOSITORY}:$(version_from_tag "${TARGET_RELEASE_TAG}")}"
if [ "\${1:-start}" = "update" ]; then
    exec bash -c "curl -sSL https://raw.githubusercontent.com/\${AGENTARK_RELEASE_REPO}/main/scripts/install.sh | AGENTARK_ASSUME_YES=1 bash"
fi
if [ "\$#" -eq 0 ]; then
    set -- start
fi
cd "\${AGENTARK_DIR}/runtime"
exec bash "\${AGENTARK_DIR}/runtime/scripts/start.sh" "\$@"
SCRIPT_EOF
    chmod +x "${INSTALL_DIR}/agentark"

    if [ -d /usr/local/bin ] && [ -w /usr/local/bin ]; then
        ln -sf "${INSTALL_DIR}/agentark" /usr/local/bin/agentark
        echo -e "${GREEN}Installed 'agentark' command globally.${NC}"
    elif command -v sudo >/dev/null 2>&1; then
        sudo ln -sf "${INSTALL_DIR}/agentark" /usr/local/bin/agentark
        echo -e "${GREEN}Installed 'agentark' command globally.${NC}"
    else
        echo -e "${YELLOW}Add ${INSTALL_DIR} to PATH to use the 'agentark' command.${NC}"
    fi
}

compose_project_name() {
    if [ -n "${COMPOSE_PROJECT_NAME:-}" ]; then
        printf '%s' "${COMPOSE_PROJECT_NAME}"
        return
    fi
    local existing
    existing="$(docker ps -a --filter "name=^/agentark-control$" --format '{{.Label "com.docker.compose.project"}}' 2>/dev/null | head -n 1 || true)"
    if [ -n "${existing}" ]; then
        printf '%s' "${existing}"
    else
        printf '%s' "agentark"
    fi
}

echo ""
echo -e "${BOLD}=========================================${NC}"
echo -e "${BOLD}  AgentArk Installer${NC}"
echo -e "  Docker image install, no source clone."
echo -e "${BOLD}=========================================${NC}"
echo ""

ensure_docker
echo -e "${GREEN}[1/4] Docker is ready.${NC}"

mkdir -p "${INSTALL_DIR}"
TARGET_RELEASE_TAG="${AGENTARK_RELEASE_TAG:-$(latest_release_tag)}"
if [ -z "${TARGET_RELEASE_TAG}" ]; then
    echo -e "${RED}Unable to resolve the latest AgentArk release.${NC}"
    exit 1
fi

download_runtime_files
write_agentark_command
echo -e "${GREEN}[2/4] Runtime files ready at ${RUNTIME_DIR}.${NC}"

export AGENTARK_IMAGE="${IMAGE_REPOSITORY}:$(version_from_tag "${TARGET_RELEASE_TAG}")"
export AGENTARK_RELEASE_REPO="${RELEASE_REPO}"
export AGENTARK_RELEASE_TAG="${TARGET_RELEASE_TAG}"
export COMPOSE_PROJECT_NAME="$(compose_project_name)"

cd "${RUNTIME_DIR}"
warn_if_port_in_use "${AGENTARK_POSTGRES_PORT:-5432}" "Postgres"
warn_if_port_in_use "8990" "AgentArk Web UI"

echo -e "${CYAN}[3/4] Pulling AgentArk image ${AGENTARK_IMAGE}...${NC}"
docker compose -p "${COMPOSE_PROJECT_NAME}" pull postgres agentark-control agentark-embeddings agentark-executor agentark-workspace

echo -e "${GREEN}[4/4] Starting AgentArk...${NC}"
docker compose -p "${COMPOSE_PROJECT_NAME}" up -d

echo ""
echo -e "${BOLD}=========================================${NC}"
echo -e "${GREEN}  AgentArk is running!${NC}"
echo -e "${BOLD}=========================================${NC}"
echo ""
echo -e "  Web UI:  ${CYAN}http://localhost:8990${NC}"
echo -e "  Image:   ${CYAN}${AGENTARK_IMAGE}${NC}"
echo ""
echo -e "  Commands:"
echo -e "    ${BOLD}agentark logs${NC}"
echo -e "    ${BOLD}agentark status${NC}"
echo -e "    ${BOLD}agentark stop${NC}"
echo -e "    ${BOLD}agentark update${NC}"
echo ""
echo -e "${YELLOW}App data is stored in Docker volumes and survives updates.${NC}"
