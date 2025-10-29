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

if [[ -n "${MERMAID_CLI_PATH:-}" ]]; then
    if [[ ! -x "${MERMAID_CLI_PATH}" ]]; then
        echo "MERMAID_CLI_PATH is set to '${MERMAID_CLI_PATH}', but it is not executable." >&2
        exit 1
    fi
else
    if ! command -v mmdc >/dev/null 2>&1; then
        echo "⚠️  Warning: Mermaid CLI (mmdc) not found in PATH. Install it with 'npm install -g @mermaid-js/mermaid-cli' to enable diagram rendering." >&2
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

rustup target add wasm32-wasip2 >/dev/null 2>&1 || true

echo "Building extension binaries..."
cargo build --lib --target wasm32-wasip2 --release
cargo build --package mermaid-lsp --release

cp target/wasm32-wasip2/release/mermaid_preview.wasm ./extension.wasm
cp target/release/mermaid-lsp ./

popd >/dev/null

echo
echo "✅ Installation complete!"
echo
echo "Restart Zed to start using the extension."
