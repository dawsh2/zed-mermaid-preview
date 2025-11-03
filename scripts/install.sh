#!/bin/bash

# Installation script for Mermaid Preview extension

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXTENSIONS_ROOT="${ZED_EXTENSIONS_DIR:-$HOME/.config/zed/extensions}"
TARGET_DIR="${EXTENSIONS_ROOT}/mermaid-preview"

echo "Installing Mermaid Preview extension for Zed..."
echo "Source: $SCRIPT_DIR"
echo "Target: $TARGET_DIR"

mkdir -p "$EXTENSIONS_ROOT"

# Check and install Mermaid CLI if needed
echo "🔍 Checking Mermaid CLI availability..."
if [[ -n "${MERMAID_CLI_PATH:-}" ]]; then
    if [[ ! -x "${MERMAID_CLI_PATH}" ]]; then
        echo "❌ MERMAID_CLI_PATH is set to '${MERMAID_CLI_PATH}', but it is not executable." >&2
        exit 1
    else
        echo "✅ Mermaid CLI found via MERMAID_CLI_PATH: ${MERMAID_CLI_PATH}"
    fi
else
    if ! command -v mmdc >/dev/null 2>&1; then
        echo "⚠️  Mermaid CLI (mmdc) not found in PATH."
        echo "🔧 Attempting to install automatically..."

        # Check if npm is available
        if command -v npm >/dev/null 2>&1; then
            echo "📦 Installing @mermaid-js/mermaid-cli globally..."
            if npm install -g @mermaid-js/mermaid-cli; then
                echo "✅ Mermaid CLI installed successfully!"
            else
                echo "❌ Failed to install Mermaid CLI automatically." >&2
                echo "Please install manually: npm install -g @mermaid-js/mermaid-cli" >&2
                read -p "Continue anyway? Diagrams won't render without it. [y/N]: " -n 1 -r
                echo
                if [[ ! $REPLY =~ ^[Yy]$ ]]; then
                    exit 1
                fi
            fi
        else
            echo "❌ npm not found. Please install Node.js and npm first." >&2
            echo "Then install Mermaid CLI: npm install -g @mermaid-js/mermaid-cli" >&2
            read -p "Continue anyway? Diagrams won't render without it. [y/N]: " -n 1 -r
            echo
            if [[ ! $REPLY =~ ^[Yy]$ ]]; then
                exit 1
            fi
        fi
    else
        echo "✅ Mermaid CLI found in PATH"
    fi
fi

echo "Copying extension files..."
rsync -a --delete \
    --exclude '.git/' \
    --exclude 'target/' \
    --exclude 'install.sh' \
    --exclude 'build.sh' \
    "$SCRIPT_DIR/" "$TARGET_DIR/"

pushd "$TARGET_DIR" >/dev/null

echo "🔧 Ensuring Rust target is available..."
if ! rustup target add wasm32-wasip2 2>/dev/null; then
    echo "⚠️  Warning: Failed to add wasm32-wasip2 target. Build may fail." >&2
fi

echo "🔨 Building extension components..."

# Build WebAssembly extension
echo "  📦 Building WebAssembly extension..."
if ! cargo build --lib --target wasm32-wasip2 --release; then
    echo "❌ Failed to build WebAssembly extension." >&2
    echo "This might be due to missing dependencies or Rust toolchain issues." >&2
    popd >/dev/null
    exit 1
fi

# Build LSP server
echo "  📦 Building LSP server..."
if ! cargo build --package mermaid-lsp --release; then
    echo "❌ Failed to build LSP server." >&2
    echo "This might be due to missing dependencies or compilation errors." >&2
    popd >/dev/null
    exit 1
fi

# Copy binaries to extension root
echo "📋 Installing binaries..."
if [[ -f "target/wasm32-wasip2/release/mermaid_preview.wasm" ]]; then
    cp target/wasm32-wasip2/release/mermaid_preview.wasm ./extension.wasm
    echo "  ✅ WebAssembly extension installed"
else
    echo "❌ WebAssembly binary not found" >&2
    popd >/dev/null
    exit 1
fi

if [[ -f "target/release/mermaid-lsp" ]]; then
    cp target/release/mermaid-lsp ./
    echo "  ✅ LSP binary installed"
else
    echo "❌ LSP binary not found" >&2
    popd >/dev/null
    exit 1
fi

popd >/dev/null

echo
echo "🎉 Installation complete!"
echo
echo "📋 What was installed:"
echo "  ✅ Mermaid Preview extension (WebAssembly)"
echo "  ✅ Mermaid LSP server (Rust binary)"
echo "  ✅ Mermaid CLI dependency (auto-installed if needed)"
echo
echo "🚀 Benefits of this installation:"
echo "  • Zero delay on first file open (binary pre-built)"
echo "  • Automatic dependency management"
echo "  • Works offline after installation"
echo "  • Progressive error handling and recovery"
echo
echo "📝 Usage:"
echo "  1. Restart Zed"
echo "  2. Open a Markdown file with Mermaid diagrams"
echo "  3. Right-click in a mermaid code block and select 'Render Mermaid Diagram'"
echo
echo "🔧 For development:"
echo "  Set MERMAID_LSP_PATH to use a custom binary"
echo "  Set MERMAID_CLI_PATH to use a custom mmdc binary"
