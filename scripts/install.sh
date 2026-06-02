#!/bin/bash
# AgentArk Installer
#
# Usage: curl -sSL https://raw.githubusercontent.com/agentark-ai/AgentArk/main/scripts/install.sh | bash

set -euo pipefail

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
BOLD='\033[1m'
NC='\033[0m'

INSTALL_DIR="${HOME}/agentark"
SOURCE_DIR="${INSTALL_DIR}/source"
RELEASE_REPO="${AGENTARK_RELEASE_REPO:-agentark-ai/AgentArk}"
REPO_URL="https://github.com/${RELEASE_REPO}.git"
IMAGE_REPOSITORY="${AGENTARK_IMAGE_REPOSITORY:-ghcr.io/agentark-ai/agentark}"

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
        echo -e "${YELLOW}Added ${USER} to the docker group. Log out and back in if Docker still needs sudo.${NC}"
    fi
}

docker_git() {
    docker run --rm -v "${INSTALL_DIR}:/work" -w /work alpine/git "$@"
}

latest_release_tag() {
    docker run --rm alpine/git ls-remote --tags --refs "${REPO_URL}" "v*" 2>/dev/null \
        | awk '{print $2}' \
        | sed 's#refs/tags/##' \
        | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
        | awk -F'[v.]' '{printf("%09d.%09d.%09d %s\n", $2, $3, $4, $0)}' \
        | sort \
        | tail -n 1 \
        | awk '{print $2}'
}

release_version_from_tag() {
    printf '%s' "${1#v}"
}

ensure_clean_checkout() {
    local tracked_changes
    tracked_changes="$(docker_git git -C /work/source status --porcelain --untracked-files=no 2>/dev/null || true)"
    if [ -n "${tracked_changes}" ]; then
        echo -e "${RED}Tracked local changes were found in ${SOURCE_DIR}. Resolve them before reinstalling.${NC}"
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
        echo -e "${YELLOW}Warning: TCP port ${port} is already in use. ${service} may fail to start unless you stop the existing listener or override the port.${NC}"
    fi
}

verify_lightpanda_runtime() {
    local attempts=20
    echo -e "${CYAN}Verifying bundled Lightpanda runtime...${NC}"
    while [ "${attempts}" -gt 0 ]; do
        if docker compose exec -T agentark-control sh -lc 'command -v lightpanda >/dev/null 2>&1' >/dev/null 2>&1; then
            echo -e "${GREEN}Lightpanda is available inside the AgentArk runtime.${NC}"
            return 0
        fi
        attempts=$((attempts - 1))
        sleep 2
    done
    echo -e "${RED}Lightpanda is missing from the bundled AgentArk runtime. Update or rebuild before relying on the free search fallback.${NC}"
    return 1
}

verify_gepa_optimizer_runtime() {
    local attempts=20
    echo -e "${CYAN}Verifying bundled GEPA optimizer runtime...${NC}"
    while [ "${attempts}" -gt 0 ]; do
        if docker compose exec -T agentark-control sh -lc '/opt/agentark-gepa/bin/python -c "import dspy" >/dev/null 2>&1' >/dev/null 2>&1; then
            echo -e "${GREEN}GEPA optimizer is available inside the AgentArk runtime.${NC}"
            return 0
        fi
        attempts=$((attempts - 1))
        sleep 2
    done
    echo -e "${RED}GEPA optimizer is missing from the bundled AgentArk runtime. Update or rebuild the image before running ArkEvolve GEPA.${NC}"
    return 1
}

pull_runtime_images() {
    ${COMPOSE} pull postgres agentark-control agentark-embeddings agentark-executor agentark-workspace
}

if command -v docker >/dev/null 2>&1; then
    echo -e "${GREEN}[1/4] Docker found.${NC}"
else
    install_docker
    echo -e "${GREEN}[1/4] Docker installed.${NC}"
fi

if docker compose version >/dev/null 2>&1; then
    COMPOSE="docker compose"
elif command -v docker-compose >/dev/null 2>&1; then
    COMPOSE="docker-compose"
else
    echo -e "${RED}docker compose not found. Please install Docker Compose v2.${NC}"
    exit 1
fi
echo -e "${GREEN}[2/4] Docker Compose found.${NC}"

mkdir -p "${INSTALL_DIR}"
TARGET_RELEASE_TAG="${AGENTARK_RELEASE_TAG:-$(latest_release_tag)}"
if [ -z "${TARGET_RELEASE_TAG}" ]; then
    echo -e "${RED}Unable to resolve the latest tagged AgentArk release.${NC}"
    exit 1
fi

if [ ! -d "${SOURCE_DIR}/.git" ]; then
    echo -e "${CYAN}Cloning AgentArk ${TARGET_RELEASE_TAG} into ${SOURCE_DIR}...${NC}"
    docker_git clone --branch "${TARGET_RELEASE_TAG}" --depth 1 "${REPO_URL}" source
else
    echo -e "${GREEN}Existing source checkout found at ${SOURCE_DIR}${NC}"
    ensure_clean_checkout
    docker_git git -C /work/source fetch --tags --force origin
    docker_git git -C /work/source checkout --force "${TARGET_RELEASE_TAG}"
fi

if [ ! -f "${SOURCE_DIR}/docker-compose.yml" ]; then
    echo -e "${RED}Missing ${SOURCE_DIR}/docker-compose.yml after checkout.${NC}"
    exit 1
fi

AGENTARK_IMAGE="${IMAGE_REPOSITORY}:$(release_version_from_tag "${TARGET_RELEASE_TAG}")"
export AGENTARK_IMAGE AGENTARK_RELEASE_REPO="${RELEASE_REPO}" AGENTARK_RELEASE_TAG="${TARGET_RELEASE_TAG}"
echo -e "${GREEN}[3/4] Source checkout ready at ${SOURCE_DIR}${NC}"

cat > "${INSTALL_DIR}/agentark" << 'SCRIPT_EOF'
#!/bin/bash
set -e
SCRIPT_PATH="$(readlink -f "$0" 2>/dev/null || realpath "$0" 2>/dev/null || echo "$0")"
AGENTARK_DIR="$(cd "$(dirname "$SCRIPT_PATH")" && pwd)"
exec bash "$AGENTARK_DIR/source/scripts/agentark-release-cli.sh" "$@"
SCRIPT_EOF
chmod +x "${INSTALL_DIR}/agentark"

if [ -d /usr/local/bin ] && [ -w /usr/local/bin ]; then
    ln -sf "${INSTALL_DIR}/agentark" /usr/local/bin/agentark
    echo -e "${GREEN}Installed 'agentark' command globally.${NC}"
elif command -v sudo >/dev/null 2>&1; then
    sudo ln -sf "${INSTALL_DIR}/agentark" /usr/local/bin/agentark
    echo -e "${GREEN}Installed 'agentark' command globally.${NC}"
else
    echo -e "${YELLOW}Could not install to /usr/local/bin. Add ${INSTALL_DIR} to your PATH:${NC}"
    echo -e "  export PATH=\"${INSTALL_DIR}:\$PATH\""
fi

echo -e "${CYAN}Downloading AgentArk container image for ${TARGET_RELEASE_TAG}...${NC}"
cd "${SOURCE_DIR}"
POSTGRES_PORT="${AGENTARK_POSTGRES_PORT:-5432}"
warn_if_port_in_use "${POSTGRES_PORT}" "Postgres"
warn_if_port_in_use "8990" "AgentArk Web UI"

echo -e "${GREEN}[4/4] Starting AgentArk...${NC}"
pull_runtime_images
${COMPOSE} up -d
verify_lightpanda_runtime
verify_gepa_optimizer_runtime

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
echo -e "    ${BOLD}agentark update${NC}     Install the latest tagged release and restart"
echo -e "    ${BOLD}agentark logs${NC}       View logs"
echo -e "    ${BOLD}agentark status${NC}     Show status"
echo -e "    ${BOLD}agentark backup${NC}     Backup Docker volumes"
echo ""
echo -e "${YELLOW}Compose checkout: ${SOURCE_DIR}${NC}"
echo -e "${YELLOW}App data is stored in Docker volumes and survives updates.${NC}"
echo -e "${YELLOW}Postgres and install secrets live in Docker volumes; use 'agentark backup' before moving installs.${NC}"
echo -e "${YELLOW}Use 'docker compose down -v' only for a full reset.${NC}"
echo ""
