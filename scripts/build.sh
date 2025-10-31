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

echo ""
echo "✅ Build complete!"
echo ""
echo "📦 Binaries built:"
echo "  - target/wasm32-wasip2/release/mermaid_preview.wasm → extension.wasm"
echo "  - target/release/mermaid-lsp → mermaid-lsp (~3.5MB)"
echo ""
echo "🔧 For development:"
echo "  Set MERMAID_LSP_PATH to use your local build:"
echo "  export MERMAID_LSP_PATH=\"$(realpath "$SCRIPT_DIR/../target/release/mermaid-lsp")\""
echo ""
echo "  Or run: ./scripts/dev-setup.sh (one-time setup)"
echo ""
echo "📦 For production users:"
echo "  Extension auto-downloads versioned binary from GitHub releases"
echo "  No manual installation needed!"
