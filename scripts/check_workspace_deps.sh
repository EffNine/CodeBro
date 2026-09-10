#!/usr/bin/env bash
# Dependency-direction guard for the CodeBro workspace.
#
# Enforces:  mcp-server -> runtime services -> parsers/core
# No service may depend on mcp-server; no circular dependencies.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
fail=0

allowed() {
    case "$1" in
        codebro-core)              echo "" ;;
        codebro-parsers)           echo "codebro-core" ;;
        codebro-fact-store)        echo "codebro-core" ;;
        codebro-identity-runtime)  echo "codebro-core" ;;
        codebro-memory-runtime)    echo "codebro-core codebro-identity-runtime" ;;
        codebro-sandbox-runtime)   echo "codebro-core" ;;
        codebro-impact-engine)     echo "codebro-core codebro-parsers codebro-fact-store" ;;
        codebro-indexer)           echo "codebro-core codebro-parsers codebro-fact-store codebro-identity-runtime codebro-impact-engine" ;;
        codebro-change-engine)     echo "codebro-core" ;;
        codebro-context-runtime)   echo "codebro-core" ;;
        codebro-mcp-server)        echo "codebro-core codebro-parsers codebro-fact-store codebro-identity-runtime codebro-memory-runtime codebro-sandbox-runtime codebro-impact-engine codebro-indexer codebro-change-engine codebro-context-runtime" ;;
        *)                         echo "" ;;
    esac
}

for manifest in "$root"/crates/*/Cargo.toml; do
    crate=$(basename "$(dirname "$manifest")")
    pkg="codebro-$crate"
    deps=$(grep -oE '^codebro-[a-z-]+\.workspace = true' "$manifest" | sed 's/\.workspace = true//') || true
    for dep in $deps; do
        if ! allowed "$pkg" | grep -qw "$dep"; then
            echo "VIOLATION: $pkg depends on $dep (not in allowed set)"
            fail=1
        fi
    done
done

if [ "$fail" -eq 0 ]; then
    echo "dependency direction OK"
fi
exit $fail
