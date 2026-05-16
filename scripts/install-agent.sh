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

# Check if Rust/Cargo is installed (check both root and user paths)
CARGO_CMD=""
if command -v cargo &> /dev/null; then
    CARGO_CMD="cargo"
    echo -e "${GREEN}✓ Cargo found (system)${NC}"
elif [ -n "$SUDO_USER" ] && [ -f "/home/$SUDO_USER/.cargo/bin/cargo" ]; then
    CARGO_CMD="sudo -u $SUDO_USER /home/$SUDO_USER/.cargo/bin/cargo"
    echo -e "${GREEN}✓ Cargo found (user: $SUDO_USER)${NC}"
else
    echo -e "${RED}✗ Cargo not found${NC}"
    echo "Shipwright agent requires Rust to build."
    echo "Install Rust from: https://rustup.rs"
    echo ""
    echo "Run: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

# Check if Docker is installed
if ! command -v docker &> /dev/null; then
    echo -e "${RED}✗ Docker not found${NC}"
    echo "Please install Docker first: https://docs.docker.com/engine/install/"
    exit 1
fi

echo -e "${GREEN}✓ Docker found${NC}"

# Determine user home directory
if [ -n "$SUDO_USER" ]; then
    USER_HOME=$(getent passwd "$SUDO_USER" | cut -d: -f6)
else
    USER_HOME="$HOME"
fi

REPO_DIR="$USER_HOME/.shipwright/repo"

echo "📂 Repository directory: $REPO_DIR"

# Clone or update repository (as the user)
if [ -d "$REPO_DIR" ]; then
    echo "📦 Updating Shipwright repository..."
    if [ -n "$SUDO_USER" ]; then
        sudo -u $SUDO_USER git -C "$REPO_DIR" pull origin main
    else
        git -C "$REPO_DIR" pull origin main
    fi
else
    echo "📦 Cloning Shipwright repository..."
    if [ -n "$SUDO_USER" ]; then
        sudo -u $SUDO_USER mkdir -p "$USER_HOME/.shipwright"
        sudo -u $SUDO_USER git clone https://github.com/tinomupezeni/shipwright.git "$REPO_DIR"
    else
        mkdir -p "$USER_HOME/.shipwright"
        git clone https://github.com/tinomupezeni/shipwright.git "$REPO_DIR"
    fi
fi

# Build the agent binary (as the user who has Rust installed, in their own directory)
echo "🔨 Building Shipwright agent binary..."
cd "$REPO_DIR"
$CARGO_CMD build --release --package shipwright-agent

if [ ! -f "target/release/shipwright-agent" ]; then
    echo -e "${RED}✗ Build failed - binary not found${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Binary built successfully${NC}"

# Install binary
echo "📦 Installing agent binary..."
cp "$REPO_DIR/target/release/shipwright-agent" /usr/local/bin/
chmod +x /usr/local/bin/shipwright-agent

# Create required directories
mkdir -p /var/lib/shipwright
mkdir -p /etc/shipwright
if [ -n "$SUDO_USER" ]; then
    mkdir -p /home/$SUDO_USER/apps
    chown -R $SUDO_USER:$SUDO_USER /home/$SUDO_USER/apps
fi

# Install systemd service
echo "⚙️  Installing systemd service..."
cp "$REPO_DIR/scripts/shipwright-agent.service" /etc/systemd/system/

# Update service file with actual paths (replace %u placeholder with actual username)
if [ -n "$SUDO_USER" ]; then
    sed -i "s|/home/%u/apps|/home/$SUDO_USER/apps|g" /etc/systemd/system/shipwright-agent.service
    sed -i "s|/home/%u/.shipwright/repo|$REPO_DIR|g" /etc/systemd/system/shipwright-agent.service
fi

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
