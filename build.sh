#!/bin/bash

# Build script for Mermaid Preview extension

set -e

echo "Building Mermaid Preview extension for Zed..."

# Check if mmdc is installed
if ! command -v mmdc &> /dev/null; then
    echo "Error: Mermaid CLI (mmdc) not found in PATH"
    echo "Please install it with: npm install -g @mermaid-js/mermaid-cli"
    exit 1
fi

# Build the WebAssembly extension
echo "Building WebAssembly extension..."
cargo build --lib --target wasm32-wasip2 --release

# Build the LSP server
echo "Building LSP server..."
cd lsp
cargo build --release
cd ..

# Copy the binaries to the root directory for easy installation
echo "Copying binaries to root directory..."
cp target/wasm32-wasip2/release/mermaid_preview.wasm ./extension.wasm
cp lsp/target/release/mermaid-lsp ./

echo "Build complete!"
echo ""
echo "To install:"
echo "1. Copy this entire directory to ~/.config/zed/extensions/mermaid-preview"
echo "2. Restart Zed"
echo ""
echo "Files created:"
echo "- extension.wasm (WebAssembly extension)"
echo "- mermaid-lsp (LSP server binary)"