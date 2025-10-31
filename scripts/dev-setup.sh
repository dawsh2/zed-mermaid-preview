#!/bin/bash
# Development setup script for contributors

set -e

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SHELL_CONFIG=""

# Detect shell
if [[ -n "$ZSH_VERSION" ]]; then
    SHELL_CONFIG="$HOME/.zshrc"
elif [[ -n "$BASH_VERSION" ]]; then
    SHELL_CONFIG="$HOME/.bashrc"
else
    echo "Unsupported shell. Please manually add:"
    echo "export MERMAID_LSP_PATH=\"$REPO_ROOT/target/release/mermaid-lsp\""
    exit 1
fi

# Check if already configured
if grep -q "MERMAID_LSP_PATH" "$SHELL_CONFIG" 2>/dev/null; then
    echo "✅ MERMAID_LSP_PATH already configured in $SHELL_CONFIG"
else
    echo "" >> "$SHELL_CONFIG"
    echo "# Zed Mermaid Preview development" >> "$SHELL_CONFIG"
    echo "export MERMAID_LSP_PATH=\"$REPO_ROOT/target/release/mermaid-lsp\"" >> "$SHELL_CONFIG"
    echo "✅ Added MERMAID_LSP_PATH to $SHELL_CONFIG"
fi

# Set for current session
export MERMAID_LSP_PATH="$REPO_ROOT/target/release/mermaid-lsp"

echo ""
echo "🔧 Development environment configured!"
echo ""
echo "Next steps:"
echo "  1. Restart your terminal (or run: source $SHELL_CONFIG)"
echo "  2. Build the LSP: cd lsp && cargo build --release"
echo "  3. Restart Zed"
echo ""
echo "Your local builds will now be used automatically!"
