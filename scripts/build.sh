#!/bin/bash

# Build script for Mermaid Preview extension

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "Building Mermaid Preview extension for Zed..."

# Ensure the WASM target is available
rustup target add wasm32-wasip2 >/dev/null 2>&1 || true

# Verify Mermaid CLI is available (either via MERMAID_CLI_PATH or PATH)
if [[ -n "${MERMAID_CLI_PATH:-}" ]]; then
    if [[ ! -x "${MERMAID_CLI_PATH}" ]]; then
        echo "MERMAID_CLI_PATH is set to '${MERMAID_CLI_PATH}', but it is not executable." >&2
        exit 1
    fi
else
    if ! command -v mmdc >/dev/null 2>&1; then
        echo "Warning: Mermaid CLI (mmdc) not found in PATH. Rendering will fail until it is installed." >&2
    fi
fi

echo "Building WebAssembly extension..."
cargo build --lib --target wasm32-wasip2 --release

echo "Building LSP server..."
cargo build --package mermaid-lsp --release

# Copy the WASM to root directory
echo "Copying extension.wasm to root directory..."
cp target/wasm32-wasip2/release/mermaid_preview.wasm "$SCRIPT_DIR/../extension.wasm"

# Copy LSP binary to root (not committed to git, but useful for local install)
cp target/release/mermaid-lsp "$SCRIPT_DIR/../mermaid-lsp"

# Optionally download platform-specific binary for bundling
if [[ "${BUNDLE_RELEASE_BINARY:-}" == "true" ]]; then
    echo "Attempting to download release binary for bundling..."

    # Get current platform info
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)

    # Convert architecture names
    case $ARCH in
        x86_64) ARCH="x86_64" ;;
        arm64|aarch64) ARCH="aarch64" ;;
        *) ARCH="x86_64" ;;  # fallback
    esac

    # Convert OS names
    case $OS in
        darwin) OS="apple-darwin" ;;
        linux) OS="unknown-linux-gnu" ;;
        *) OS="unknown-linux-gnu" ;;  # fallback
    esac

    BINARY_NAME="mermaid-lsp-${ARCH}-${OS}.zip"
    DOWNLOAD_DIR="$SCRIPT_DIR/../target/release-bundle"

    echo "Looking for binary: $BINARY_NAME"

    # Get latest release info
    if command -v curl >/dev/null 2>&1; then
        LATEST_RELEASE=$(curl -s https://api.github.com/repos/dawsh2/zed-mermaid-preview/releases/latest | grep '"tag_name"' | cut -d'"' -f4)
        if [[ -n "$LATEST_RELEASE" ]]; then
            echo "Latest release: $LATEST_RELEASE"

            # Create download directory
            mkdir -p "$DOWNLOAD_DIR"

            # Download the binary
            DOWNLOAD_URL="https://github.com/dawsh2/zed-mermaid-preview/releases/download/$LATEST_RELEASE/$BINARY_NAME"
            echo "Downloading from: $DOWNLOAD_URL"

            if curl -L -o "$DOWNLOAD_DIR/$BINARY_NAME" "$DOWNLOAD_URL"; then
                echo "Downloaded successfully"

                # Extract the binary
                cd "$DOWNLOAD_DIR"
                if unzip -q "$BINARY_NAME"; then
                    echo "Extracted binary"

                    # Copy to extension root for bundling
                    cp mermaid-lsp "$SCRIPT_DIR/../"
                    echo "Bundled release binary in extension root"
                else
                    echo "Failed to extract binary"
                fi
            else
                echo "Failed to download binary"
            fi
        else
            echo "Failed to get latest release info"
        fi
    else
        echo "curl not available, skipping binary download"
    fi
fi

echo ""
echo "✅ Build complete!"
echo ""
echo "📦 Binaries built:"
echo "  - target/wasm32-wasip2/release/mermaid_preview.wasm → extension.wasm"
echo "  - target/release/mermaid-lsp → mermaid-lsp (~3.5MB)"
if [[ "${BUNDLE_RELEASE_BINARY:-}" == "true" ]]; then
    echo "  - Release binary bundled in extension root (zero install time!)"
fi
echo ""
echo "🔧 For development:"
echo "  Set MERMAID_LSP_PATH to use your local build:"
echo "  export MERMAID_LSP_PATH=\"$(realpath "$SCRIPT_DIR/../target/release/mermaid-lsp")\""
echo ""
echo "  Or run: ./scripts/dev-setup.sh (one-time setup)"
echo ""
echo "📦 For production builds:"
echo "  BUNDLE_RELEASE_BINARY=true ./scripts/build.sh  # Bundle release binary"
echo ""
echo "📦 For production users:"
echo "  Extension auto-downloads versioned binary from GitHub releases"
echo "  Or uses bundled binary if available during installation"
