#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <target-triple>" >&2
    exit 1
fi

TARGET="$1"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SCRIPT_DIR%/scripts}"
REPO_ROOT="${REPO_ROOT:-$SCRIPT_DIR}"

cd "$REPO_ROOT"

echo "==> Packaging mermaid-lsp for target '$TARGET'"

rustup target add "$TARGET"
cargo build --package mermaid-lsp --release --target "$TARGET"

EXT=""
if [[ "$TARGET" == *"windows"* ]]; then
    EXT=".exe"
fi

BIN_PATH="target/${TARGET}/release/mermaid-lsp${EXT}"

if [[ ! -f "$BIN_PATH" ]]; then
    echo "Expected binary not found at $BIN_PATH" >&2
    exit 1
fi

DIST_DIR="$REPO_ROOT/release-artifacts"
mkdir -p "$DIST_DIR"

ARCHIVE_NAME="mermaid-lsp-${TARGET}.zip"
ARCHIVE_PATH="$DIST_DIR/$ARCHIVE_NAME"

echo "==> Creating archive $ARCHIVE_PATH"

python3 - "$BIN_PATH" "$ARCHIVE_PATH" <<'PY'
import os
import sys
import zipfile

bin_path = os.path.abspath(sys.argv[1])
archive_path = os.path.abspath(sys.argv[2])

with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
    archive.write(bin_path, arcname=os.path.basename(bin_path))
PY

echo "Archive ready: $ARCHIVE_PATH"
