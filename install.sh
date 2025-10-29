#!/bin/bash

# Installation script for Mermaid Preview extension

set -e

# Get the directory where this script is located
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
EXTENSION_DIR="$HOME/.config/zed/extensions/mermaid-preview"

echo "Installing Mermaid Preview extension for Zed..."
echo "Source: $SCRIPT_DIR"
echo "Target: $EXTENSION_DIR"

# Check if mmdc is installed
if ! command -v mmdc &> /dev/null; then
    echo ""
    echo "⚠️  Warning: Mermaid CLI (mmdc) not found in PATH"
    echo "   Please install it with: npm install -g @mermaid-js/mermaid-cli"
    echo ""
    read -p "Continue anyway? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

# Create the extension directory if it doesn't exist
mkdir -p "$EXTENSION_DIR"

# Copy all necessary files
echo "Copying extension files..."
cp -r "$SCRIPT_DIR"/* "$EXTENSION_DIR/"

# Remove the install scripts from the installed extension
rm -f "$EXTENSION_DIR/install.sh" "$EXTENSION_DIR/build.sh"

# Build the extension
echo "Building extension..."
cd "$EXTENSION_DIR"
cargo build --lib --target wasm32-wasip2 --release
cd lsp
cargo build --release
cd ..

# Copy binaries to root
cp target/wasm32-wasip2/release/mermaid_preview.wasm ./extension.wasm
cp lsp/target/release/mermaid-lsp ./

echo ""
echo "✅ Installation complete!"
echo ""
echo "Restart Zed to start using the extension."
echo ""
echo "Usage:"
echo "1. Create a mermaid code block in markdown"
echo "2. Right-click and select 'Render Mermaid Diagram'"