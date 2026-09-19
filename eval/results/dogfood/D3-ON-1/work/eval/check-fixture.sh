#!/usr/bin/env bash
# Fixture self-check (Phase-0 finding F-01 structural rule).
#
# For every eval/tasks/<T>/, verifies:
#  1. setup.sh and grade.sh exist and are executable.
#  2. setup.sh installs task.md, and every other doc shipped in the task dir
#     that trial prompts may reference (spec.md, NOTES.md, continuation.md).
#     (F-01: the Phase-0 T3-B prompt cited continuation.md, which setup.sh
#     never installed. No prompt may reference a file setup.sh does not ship.)
#  3. setup.sh never touches hidden tests (no 'hidden' string; agent-visible
#     tree must not contain hidden material before grading).
#  4. Grading the fresh stub FAILS (the task is non-trivial; a green stub
#     means the fixture cannot measure anything).
#
# Usage: eval/check-fixture.sh  (run from repo root; exit nonzero on failure)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
fail=0

for tdir in "$root"/eval/tasks/*/; do
  t="$(basename "$tdir")"
  echo "=== $t ==="
  # 1. executable scripts
  for s in setup.sh grade.sh; do
    if [ ! -x "$tdir/$s" ]; then echo "FAIL: $t/$s missing or not executable"; fail=1; fi
  done
  # 2. docs shipped
  [ -f "$tdir/task.md" ] || { echo "FAIL: $t/task.md missing"; fail=1; }
  for doc in spec.md NOTES.md continuation.md; do
    if [ -f "$tdir/$doc" ]; then
      scratch="$(mktemp -d)"
      "$tdir/setup.sh" "$scratch/fix" >/dev/null 2>&1
      if [ ! -f "$scratch/fix/$doc" ]; then
        echo "FAIL (F-01): $t/$doc exists in fixture but setup.sh does not install it"
        fail=1
      fi
      rm -rf "$scratch"
    fi
  done
  # 3. no hidden leak in the installed tree (comments in setup.sh may name
  #    the file; what matters is the agent-visible tree must not contain it)
  scratch3="$(mktemp -d)"
  "$tdir/setup.sh" "$scratch3/fix" >/dev/null 2>&1
  if grep -r -i "hidden" "$scratch3/fix" >/dev/null 2>&1; then
    echo "FAIL: installed $t tree contains hidden-test material:"
    grep -r -i -l "hidden" "$scratch3/fix"
    fail=1
  fi
  rm -rf "$scratch3"
  # 4. stub must fail grading
  scratch="$(mktemp -d)"
  "$tdir/setup.sh" "$scratch/fix" >/dev/null 2>&1
  if "$tdir/grade.sh" "$scratch/fix" >/dev/null 2>&1; then
    echo "FAIL: $t stub passes grading (fixture measures nothing)"
    fail=1
  else
    echo "ok: $t stub fails grading as designed"
  fi
  rm -rf "$scratch"
done

if [ "$fail" -ne 0 ]; then echo "CHECK-FIXTURE: FAILURES PRESENT"; exit 1; fi
echo "CHECK-FIXTURE: all fixtures structurally sound"
