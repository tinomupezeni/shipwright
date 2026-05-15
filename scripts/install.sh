#!/bin/bash

# Shipwright Installer
# Usage: curl -fsSL https://raw.githubusercontent.com/tinomupezeni/shipwright/main/scripts/install.sh | bash

set -e

REPO="tinomupezeni/shipwright" # Replace with your actual username/repo
GITHUB_URL="https://github.com/$REPO"

# Detect OS
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)
        TARGET="x86_64-unknown-linux-gnu"
        ;;
    Darwin)
        if [ "$ARCH" = "arm64" ]; then
            TARGET="aarch64-apple-darwin"
        else
            TARGET="x86_64-apple-darwin"
        fi
        ;;
    *)
        echo "Unsupported OS: $OS"
        exit 1
        ;;
esac

echo "🚀 Installing Shipwright for $TARGET..."

# Get latest release version
LATEST_VERSION=$(curl -s "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')

if [ -z "$LATEST_VERSION" ]; then
    echo "❌ Failed to find the latest release. Make sure you have created a release on GitHub."
    exit 1
fi

echo "📦 Downloading Shipwright $LATEST_VERSION..."
DOWNLOAD_URL="$GITHUB_URL/releases/download/$LATEST_VERSION/shipwright-$TARGET.tar.gz"

curl -L "$DOWNLOAD_URL" -o shipwright.tar.gz

# Extract
tar -xzf shipwright.tar.gz

# Install
echo "🔧 Installing binaries to /usr/local/bin..."
sudo mv shipwright-cli /usr/local/bin/shipwright
sudo mv shipwright-agent /usr/local/bin/shipwright-agent

# Cleanup
rm shipwright.tar.gz

echo "✅ Shipwright installed successfully!"
echo "Run 'shipwright --help' to get started."
