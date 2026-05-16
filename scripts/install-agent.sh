#!/bin/bash
# Shipwright Agent Installation Script (Docker-based)
# Installs the Shipwright agent as a Docker container for easy updates and management

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}"
echo "  ____  _     _                      _       _     _   "
echo " / ___|| |__ (_)_ ____      ___ __ (_) __ _| |__ | |_ "
echo " \___ \| '_ \| | '_ \ \ /\ / / '__| |/ _\` | '_ \| __|"
echo "  ___) | | | | | |_) \ V  V /| |  | | (_| | | | | |_ "
echo " |____/|_| |_|_| .__/ \_/\_/ |_|  |_|\__, |_| |_|\__|"
echo "               |_|                   |___/            "
echo -e "${NC}"
echo "Shipwright Agent - Docker Installation"
echo ""

# Check if Docker is installed
if ! command -v docker &> /dev/null; then
    echo -e "${RED}✗ Docker not found${NC}"
    echo "Please install Docker first: https://docs.docker.com/engine/install/"
    exit 1
fi

echo -e "${GREEN}✓ Docker found${NC}"

# Check if docker-compose is available
if ! command -v docker-compose &> /dev/null && ! docker compose version &> /dev/null; then
    echo -e "${YELLOW}⚠ docker-compose not found, will use 'docker compose'${NC}"
    COMPOSE_CMD="docker compose"
else
    COMPOSE_CMD="docker-compose"
fi

# Create installation directory
INSTALL_DIR="$HOME/.shipwright"
mkdir -p "$INSTALL_DIR"

echo "📂 Installation directory: $INSTALL_DIR"

# Clone or update repository
if [ -d "$INSTALL_DIR/repo" ]; then
    echo "📦 Updating Shipwright repository..."
    cd "$INSTALL_DIR/repo"
    git pull origin main
else
    echo "📦 Cloning Shipwright repository..."
    git clone https://github.com/tinomupezeni/shipwright.git "$INSTALL_DIR/repo"
    cd "$INSTALL_DIR/repo"
fi

# Copy docker-compose file
cp docker-compose.agent.yml "$INSTALL_DIR/docker-compose.yml"

# Create .env file for configuration
if [ ! -f "$INSTALL_DIR/.env" ]; then
    cat > "$INSTALL_DIR/.env" << EOF
# Shipwright Agent Configuration
RUST_LOG=info
HOME=$HOME
USER=$USER
SHIPWRIGHT_DEPLOY_DIR=$HOME/apps
EOF
    echo -e "${GREEN}✓ Created configuration file${NC}"
fi

# Create apps directory
mkdir -p "$HOME/apps"

# Create or join proxy network
if ! docker network inspect proxy-tier &> /dev/null; then
    echo "🌐 Creating proxy-tier network..."
    docker network create proxy-tier
fi

# Build the agent image
echo "🔨 Building Shipwright agent Docker image..."
cd "$INSTALL_DIR/repo"
docker build -f agent/Dockerfile -t shipwright-agent:latest .

# Stop existing container if running
if docker ps -a --format '{{.Names}}' | grep -q "^shipwright-agent$"; then
    echo "🔄 Stopping existing agent container..."
    docker stop shipwright-agent || true
    docker rm shipwright-agent || true
fi

# Start the agent
echo "🚀 Starting Shipwright agent..."
cd "$INSTALL_DIR"
$COMPOSE_CMD up -d

# Wait for health check
echo "⏳ Waiting for agent to be ready..."
sleep 5

# Check if agent is healthy
if docker ps --format '{{.Names}}\t{{.Status}}' | grep shipwright-agent | grep -q "healthy\|Up"; then
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${GREEN}✓ Shipwright Agent installed successfully!${NC}"
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo "📊 Agent Status:"
    docker ps --filter name=shipwright-agent --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"
    echo ""
    echo "📝 Next Steps:"
    echo ""
    echo "  1. Configure your domain for HTTPS webhooks (optional but recommended):"
    echo -e "     ${BLUE}shipwright setup-domain yourdomain.com${NC}"
    echo ""
    echo "  2. Register your first project:"
    echo -e "     ${BLUE}cd your-project && shipwright register${NC}"
    echo ""
    echo "  3. View agent logs:"
    echo -e "     ${BLUE}docker logs -f shipwright-agent${NC}"
    echo ""
    echo "  4. Update agent (automatic via webhook or manual):"
    echo -e "     ${BLUE}cd $INSTALL_DIR && docker-compose pull && docker-compose up -d${NC}"
    echo ""
else
    echo -e "${RED}✗ Agent failed to start properly${NC}"
    echo "Check logs with: docker logs shipwright-agent"
    exit 1
fi
