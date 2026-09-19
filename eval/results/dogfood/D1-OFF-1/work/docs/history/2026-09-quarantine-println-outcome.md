# Quarantine telemetry via println — outcome record (2026-09)

Status: REVERTED
Date: 2026-09-02
Task: observability for quarantine events
Outcome classification: failure (test-environment)

## What was attempted

`quarantine_file()` (`crates/core/src/persistence.rs`) and the context-store
quarantine path (`crates/context-runtime/src/db.rs`) were instrumented with
`println!` diagnostics: which file, quarantine destination, byte counts,
caller context. Goal: post-incident debuggability of corrupted-state
events.

## What happened

The full suite failed in the P8 E2E harness (`tests/p8_security_boundary_e2e.rs`
family and the stdio-hygiene pins). `codebro serve` reserves stdout for
JSON-RPC framing; any non-JSON-RPC line on stdout corrupts the stream and
the harness fails the suite on the first such line. The persistence layer
is serve-reachable (the memory store quarantines corrupt engineering memory
files during MCP serving), so the `println!` reached stdout in a
serve-path test and broke the contract. The change was reverted the same
day.

## Root cause

Observability was added at the wrong layer with the wrong mechanism. The
stdio-hygiene contract (see AGENTS.md "stdio hygiene (P8)" and
`docs/evolution/P8_IMPLEMENTATION.md`) predates the change: tracing (stderr)
is the only sanctioned observability channel for serve-reachable paths.
A `println!` in core persistence looks harmless in a unit test and fails
only in the serve-path integration harness — exactly the kind of failure
that is cheap to avoid with prior knowledge and expensive to discover by
test-cycle.

## Lesson (invariant)

- Persistence/quarantine code paths are serve-reachable. Observability
  there must use `tracing` (which the serve-mode subscriber routes to
  stderr), never `println!`.
- Tests that assert observability should assert on the tracing event
  stream or on behaviour, not on captured stdout.
- Scope note: two quarantine implementations exist by design — core
  persistence `quarantine_file` (JSON stores) and the context-runtime
  db quarantine (SQLite + WAL sidecars, `db.rs`). Changes to observability
  should be explicit about which one they touch; don't conflate them.
