#!/bin/bash
# Force Zed to use our local LSP binary instead of downloading from GitHub
export MERMAID_LSP_PATH="/Users/daws/repos/mermaid-preview/target/release/mermaid-lsp"
export MERMAID_LSP_DEBUG=true
echo "Environment variables set:"
echo "MERMAID_LSP_PATH=$MERMAID_LSP_PATH"
echo "MERMAID_LSP_DEBUG=$MERMAID_LSP_DEBUG"
echo ""
echo "Now restart Zed with these environment variables:"
echo "  source ./set_env.sh"
echo "  /Applications/Zed.app/Contents/MacOS/Zed"
echo ""
echo "Or quit Zed completely and restart it from this terminal."