# ADR-015: CLI and MCP doctor surfaces stay separate

Status: ACCEPTED
Date: 2026-08-14
Supersedes: none

## Context

`codebro doctor` (CLI, `crates/mcp-server/src/cli/mod.rs` `Doctor` arm)
prints a human report and exits with a status code. The MCP server exposes
the same checks through the `repository_health` tool. A machine-readable
doctor output (JSON) was requested for CI consumption.

## Decision

The CLI and the MCP tool remain separate consumers of the single check
implementation `doctor::report()` (`crates/mcp-server/src/doctor/mod.rs`).
Machine-readable CLI output, if added, is implemented as a CLI-local
serializer next to `print_report` — the CLI never routes through the MCP
server, and the MCP handler never grows CLI concerns (exit codes, printing).

## Rejected alternative

An earlier attempt wired the CLI doctor subcommand through the MCP server:
the CLI would boot an in-process MCP server instance (or invoke `codebro
serve` machinery) and consume `repository_health` output. Rejected because:

1. It introduces the stdio JSON-RPC transport and its initialization into a
   plain CLI code path. `codebro serve` reserves stdout for JSON-RPC and
   routes tracing to stderr; a CLI-embedded server instance makes the
   doctor subcommand's stdout/stderr contract ambiguous and risks
   non-JSON-RPC output leaking into pipelines that consume doctor output.
2. Composition cost: the MCP server carries the full runtime composition
   (workspaces, mutation locks, consultant providers). Doctor is a
   diagnostics path; it must be cheap, synchronous, and dependency-free.
3. Thin-handler principle (MCP_SERVER.md): handlers are thin adapters that
   delegate to canonical runtimes. Both CLI and MCP are already thin
   adapters over `report()`; keeping both thin is the point of the design.
   A CLI→MCP hop adds a layer that owns no logic and breaks the rule that
   each surface composes the runtime it needs, nothing more.

## Consequences

- `report()` stays the single source of check truth; both consumers keep
  their own output formatting.
- Any future output mode (JSON, SARIF, etc.) is added per-surface, not by
  unifying the surfaces.
- The MCP `repository_health` tool's response shape is governed by the MCP
  contract and does not track CLI output modes.
