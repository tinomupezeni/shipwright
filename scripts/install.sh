#!/bin/bash
# Shipwright Installation Script
# Works on Linux and macOS
# Usage: curl -fsSL https://raw.githubusercontent.com/tinomupezeni/shipwright/main/scripts/install.sh | bash

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
echo "Intelligent Deployment Automation for VPS"
echo ""

# Detect OS and architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux*)
        OS_TYPE="linux"
        ;;
    Darwin*)
        OS_TYPE="macos"
        ;;
    *)
        echo -e "${RED}✗ Unsupported operating system: $OS${NC}"
        echo "Shipwright currently supports Linux and macOS"
        exit 1
        ;;
esac

case "$ARCH" in
    x86_64 | amd64)
        ARCH_TYPE="x86_64"
        ;;
    aarch64 | arm64)
        ARCH_TYPE="aarch64"
        ;;
    *)
        echo -e "${RED}✗ Unsupported architecture: $ARCH${NC}"
        echo "Shipwright currently supports x86_64 and aarch64"
        exit 1
        ;;
esac

echo -e "${GREEN}✓ Detected: $OS_TYPE ($ARCH_TYPE)${NC}"

# For now, install from source since we don't have releases yet
echo "Installing from source..."

# Check if cargo is installed
if ! command -v cargo &> /dev/null; then
    echo -e "${RED}✗ Cargo not found${NC}"
    echo "Install Rust from: https://rustup.rs"
    echo ""
    echo "Run: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

TMP_DIR=$(mktemp -d)
cd "$TMP_DIR"

echo "Cloning repository..."
git clone https://github.com/tinomupezeni/shipwright.git
cd shipwright

echo "Building Shipwright CLI..."
cargo build --release --package shipwright-cli

echo "Building Shipwright Agent..."
cargo build --release --package shipwright-agent

INSTALL_DIR="$HOME/.shipwright/bin"
mkdir -p "$INSTALL_DIR"

cp target/release/shipwright "$INSTALL_DIR/"
cp target/release/shipwright-agent "$INSTALL_DIR/"

cd -
rm -rf "$TMP_DIR"

# Make binaries executable
chmod +x "$INSTALL_DIR/shipwright"
chmod +x "$INSTALL_DIR/shipwright-agent"

# Add to PATH if not already there
SHELL_CONFIG=""
case "$SHELL" in
    */bash)
        SHELL_CONFIG="$HOME/.bashrc"
        ;;
    */zsh)
        SHELL_CONFIG="$HOME/.zshrc"
        ;;
    *)
        SHELL_CONFIG="$HOME/.profile"
        ;;
esac

if [ -f "$SHELL_CONFIG" ]; then
    if ! grep -q ".shipwright/bin" "$SHELL_CONFIG"; then
        echo "" >> "$SHELL_CONFIG"
        echo "# Shipwright" >> "$SHELL_CONFIG"
        echo "export PATH=\"\$HOME/.shipwright/bin:\$PATH\"" >> "$SHELL_CONFIG"
        echo -e "${GREEN}✓ Added to PATH in $SHELL_CONFIG${NC}"
    fi
fi

echo ""
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}✓ Shipwright installed successfully!${NC}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
echo "Location: $INSTALL_DIR"
echo ""
echo "Next steps:"
echo ""
echo "  1. Reload your shell:"
echo -e "     ${BLUE}source $SHELL_CONFIG${NC}"
echo ""
echo "  2. Verify installation:"
echo -e "     ${BLUE}shipwright --version${NC}"
echo ""
echo "  3. Get started:"
echo -e "     ${BLUE}shipwright --help${NC}"
echo ""
echo "Documentation: https://github.com/tinomupezeni/shipwright"
echo ""
