#!/bin/bash
# Shipwright Agent Installation Script (Systemd)
# Installs the Shipwright agent as a native systemd service

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
echo " |____/|_| |_|_| .__/ \_/\_/ |_|  |_|\__, |_| |_|____|"
echo "               |_|                   |___/            "
echo -e "${NC}"
echo "Shipwright Agent - Systemd Installation"
echo ""

# Check if running as root or with sudo
if [[ $EUID -ne 0 ]]; then
   echo -e "${RED}✗ This script must be run as root or with sudo${NC}"
   exit 1
fi

# Check if Rust/Cargo is installed
if ! command -v cargo &> /dev/null; then
    echo -e "${RED}✗ Cargo not found${NC}"
    echo "Shipwright agent requires Rust to build."
    echo "Install Rust from: https://rustup.rs"
    echo ""
    echo "Run: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

echo -e "${GREEN}✓ Cargo found${NC}"

# Check if Docker is installed
if ! command -v docker &> /dev/null; then
    echo -e "${RED}✗ Docker not found${NC}"
    echo "Please install Docker first: https://docs.docker.com/engine/install/"
    exit 1
fi

echo -e "${GREEN}✓ Docker found${NC}"

# Create installation directory
INSTALL_DIR="/opt/shipwright"
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

# Build the agent binary
echo "🔨 Building Shipwright agent binary..."
cargo build --release --package shipwright-agent

if [ ! -f "target/release/shipwright-agent" ]; then
    echo -e "${RED}✗ Build failed - binary not found${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Binary built successfully${NC}"

# Install binary
echo "📦 Installing agent binary..."
cp target/release/shipwright-agent /usr/local/bin/
chmod +x /usr/local/bin/shipwright-agent

# Create required directories
mkdir -p /var/lib/shipwright
mkdir -p /etc/shipwright
mkdir -p /home/$SUDO_USER/apps

# Install systemd service
echo "⚙️  Installing systemd service..."
cp scripts/shipwright-agent.service /etc/systemd/system/
systemctl daemon-reload

# Stop existing service if running
if systemctl is-active --quiet shipwright-agent; then
    echo "🔄 Stopping existing agent service..."
    systemctl stop shipwright-agent
fi

# Enable and start the service
echo "🚀 Starting Shipwright agent..."
systemctl enable shipwright-agent
systemctl start shipwright-agent

# Wait for service to start
sleep 3

# Check if agent is running
if systemctl is-active --quiet shipwright-agent; then
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${GREEN}✓ Shipwright Agent installed successfully!${NC}"
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo "📊 Agent Status:"
    systemctl status shipwright-agent --no-pager -l
    echo ""
    echo "📝 Next Steps:"
    echo ""
    echo "  1. Check agent logs:"
    echo -e "     ${BLUE}journalctl -u shipwright-agent -f${NC}"
    echo ""
    echo "  2. Register your first project:"
    echo -e "     ${BLUE}cd your-project && shipwright register${NC}"
    echo ""
    echo "  3. Update agent (will auto-restart via webhook or manual):"
    echo -e "     ${BLUE}systemctl restart shipwright-agent${NC}"
    echo ""
else
    echo -e "${RED}✗ Agent failed to start properly${NC}"
    echo "Check logs with: journalctl -u shipwright-agent -n 50"
    exit 1
fi
