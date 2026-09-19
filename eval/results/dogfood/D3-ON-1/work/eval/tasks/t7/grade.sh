#!/usr/bin/env bash
# T7 grader: install held-out hidden tests into a scratch copy of the fixture,
# then run the full suite. The grader decides correctness, never the transcript.
#
# Usage: grade.sh <fixture-dir> [test-filter]
set -euo pipefail

fixture="${1:?usage: grade.sh <fixture-dir> [test-filter]}"
filter="${2:-}"
here="$(cd "$(dirname "$0")" && pwd)"

mkdir -p "$fixture/tests"
cp "$here/hidden_tests.rs" "$fixture/tests/hidden.rs"

if [ -z "$filter" ]; then
    cargo test --no-fail-fast --manifest-path "$fixture/Cargo.toml"
else
    cargo test --no-fail-fast --manifest-path "$fixture/Cargo.toml" -- "$filter"
fi
