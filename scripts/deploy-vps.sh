#!/bin/bash
# Crate Agent - VPS Deployment Script
# Deploy to a remote VPS with Docker isolation

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Configuration
VPS_HOST="${VPS_HOST:-}"
VPS_USER="${VPS_USER:-root}"
VPS_PORT="${VPS_PORT:-22}"
DEPLOY_DIR="${DEPLOY_DIR:-/opt/agentark}"

usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Deploy Crate Agent to a VPS with Docker"
    echo ""
    echo "Options:"
    echo "  -h, --host HOST     VPS hostname or IP (required)"
    echo "  -u, --user USER     SSH user (default: root)"
    echo "  -p, --port PORT     SSH port (default: 22)"
    echo "  -d, --dir DIR       Deploy directory (default: /opt/agentark)"
    echo "  --help              Show this help"
    echo ""
    echo "Environment variables:"
    echo "  VPS_HOST, VPS_USER, VPS_PORT, DEPLOY_DIR"
    echo ""
    echo "Examples:"
    echo "  $0 --host 192.168.1.100"
    echo "  $0 --host myserver.com --user deploy"
    exit 1
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -h|--host) VPS_HOST="$2"; shift 2 ;;
        -u|--user) VPS_USER="$2"; shift 2 ;;
        -p|--port) VPS_PORT="$2"; shift 2 ;;
        -d|--dir) DEPLOY_DIR="$2"; shift 2 ;;
        --help) usage ;;
        *) echo "Unknown option: $1"; usage ;;
    esac
done

if [ -z "$VPS_HOST" ]; then
    echo "Error: VPS host is required"
    usage
fi

SSH_CMD="ssh -p $VPS_PORT $VPS_USER@$VPS_HOST"
SCP_CMD="scp -P $VPS_PORT"

echo "╔═══════════════════════════════════════════════════════════╗"
echo "║          Crate Agent - VPS Deployment                     ║"
echo "╚═══════════════════════════════════════════════════════════╝"
echo ""
echo "Target: $VPS_USER@$VPS_HOST:$DEPLOY_DIR"
echo ""

# Test connection
echo "Testing SSH connection..."
$SSH_CMD "echo 'Connection successful'" || {
    echo "Failed to connect to VPS"
    exit 1
}

# Check Docker on VPS
echo "Checking Docker on VPS..."
$SSH_CMD "docker --version" || {
    echo "Docker not found. Installing..."
    $SSH_CMD "curl -fsSL https://get.docker.com | sh"
    $SSH_CMD "systemctl enable docker && systemctl start docker"
}

$SSH_CMD "docker compose version" || {
    echo "Docker Compose not found. Installing..."
    $SSH_CMD "apt-get update && apt-get install -y docker-compose-plugin"
}

# Create deployment directory
echo "Setting up deployment directory..."
$SSH_CMD "mkdir -p $DEPLOY_DIR"

# Create deployment package
echo "Creating deployment package..."
DEPLOY_FILES=(
    ".dockerignore"
    ".env.example"
    "Dockerfile"
    "docker-compose.yml"
    "Cargo.toml"
    "Cargo.lock"
    "assets"
    "docker-entrypoint.sh"
    "frontend"
    "services"
    "src"
    "config"
    "skills"
)

TEMP_DIR=$(mktemp -d)
for file in "${DEPLOY_FILES[@]}"; do
    if [ -e "$PROJECT_ROOT/$file" ]; then
        cp -r "$PROJECT_ROOT/$file" "$TEMP_DIR/"
    fi
done

# Create tarball
cd "$TEMP_DIR"
tar -czf deploy.tar.gz .

# Upload
echo "Uploading to VPS..."
$SCP_CMD "$TEMP_DIR/deploy.tar.gz" "$VPS_USER@$VPS_HOST:/tmp/"

# Extract and deploy
echo "Deploying..."
$SSH_CMD << EOF
cd $DEPLOY_DIR
tar -xzf /tmp/deploy.tar.gz
rm /tmp/deploy.tar.gz

# Stop existing containers
docker compose down 2>/dev/null || true

# Pull and start
docker compose pull
docker compose up -d

# Show status
echo ""
echo "Deployment complete!"
echo ""
docker compose ps
EOF

# Cleanup
rm -rf "$TEMP_DIR"

# Get VPS IP for display
echo ""
echo "╔═══════════════════════════════════════════════════════════╗"
echo "║                DEPLOYMENT COMPLETE!                       ║"
echo "╚═══════════════════════════════════════════════════════════╝"
echo ""
echo "Crate Agent is now running on your VPS!"
echo ""
echo "Access points:"
echo "  Web UI:  http://$VPS_HOST:8990"
echo "  API:     http://$VPS_HOST:8990/status"
echo ""
echo "Management commands (run on VPS):"
echo "  cd $DEPLOY_DIR"
echo "  docker compose logs -f          # View logs"
echo "  docker compose pull && docker compose up -d  # Update"
echo "  docker compose restart          # Restart"
echo "  docker compose down             # Stop"
echo "  docker compose up -d            # Start"
echo ""
