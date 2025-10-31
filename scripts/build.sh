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

# Copy the binaries to the root directory for easy installation
echo "Copying binaries to root directory..."
cp target/wasm32-wasip2/release/mermaid_preview.wasm "$SCRIPT_DIR/extension.wasm"
cp target/release/mermaid-lsp "$SCRIPT_DIR/"

echo "Build complete!"
echo ""
echo "To install:"
echo "1. Run ./install.sh or copy this directory into your Zed extensions folder"
echo "2. Restart Zed"
echo ""
echo "Files created:"
echo "- extension.wasm (WebAssembly extension)"
echo "- mermaid-lsp (LSP server binary)"
