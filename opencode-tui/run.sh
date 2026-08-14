#!/usr/bin/env bash
# Launch CodeBro with the OpenCode-derived TUI frontend
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CODEBRO_BIN="${CODEBRO_BIN:-$(command -v codebro)}"

if [ -z "$CODEBRO_BIN" ]; then
    echo "Error: codebro binary not found. Set CODEBRO_BIN or install codebro first."
    exit 1
fi

exec "$CODEBRO_BIN" tui
