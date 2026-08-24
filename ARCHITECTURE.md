# CodeBro Architecture

**CodeBro is an MCP-based engineering intelligence runtime that provides AI coding agents with repository understanding, verified facts, engineering memory, impact analysis, and safe execution.**

It is not a chat UI, not an agent loop, and not a model provider. The host
agent (OpenCode, Claude Code, Codex, Cursor, ...) owns planning, tool
selection and conversation; CodeBro owns engineering truth and safe execution,
exposed over MCP stdio (`codebro serve`).

Since v1.0.0 CodeBro is a 10-crate Cargo workspace; dependency direction is
enforced by `scripts/check_workspace_deps.sh`.

```
AI Coding Agent (OpenCode / Claude Code / Cursor)
        │  MCP over stdio (rmcp)
        ▼
codebro-mcp-server ────────── crates/mcp-server/src/mcp/  (17 tools)
        │
        ▼
Runtime service crates
        ├─ codebro-identity-runtime   declared intent (.codebro/project_identity.json)
        ├─ codebro-fact-store         canonical fact model + immutable validated store
        ├─ codebro-memory-runtime     persistent memory (bounded resolution, trust tiers,
        │                             lifecycle/expiry/provenance/conflict detection)
        ├─ codebro-impact-engine      relationship graph + bounded traversal with
        │                             per-edge confidence and reason
        ├─ codebro-indexer            tree-sitter indexing pipeline (multi-manifest
        │                             discovery, incremental parse cache, facts diff)
        ├─ codebro-change-engine      ChangeEngine — guarded single-file seam +
        │                             transactional multi-file apply with rollback
        └─ codebro-sandbox-runtime    policy-gated execution (Local PTY | OpenSandbox)
                                      with evidence envelopes
        │
        ▼
Foundation crates
        ├─ codebro-parsers            tree-sitter platform (Rust, Go, Python, JS, TS)
        └─ codebro-core               error, provenance, repo state, persistence,
                                      config, shell exec + patch machinery
```

## Dependency direction

The rule is strict and one-way:

```
MCP  →  Engineering Runtime  →  Core Services
```

The retired architecture (pre-MCP TUI agent stack and Adaptive Platform
subsystems, ~110k lines) was deleted in v1.0; see
`docs/LEGACY_RETIREMENT.md`. History is preserved on tags
`v0.7.0-mcp-rc1/rc2` and branch `tui-legacy`, and guards in
`crates/mcp-server/tests/legacy_isolation.rs` prevent reintroduction.

## The 17 MCP tools (frozen v1 contract)

| # | Tool | Kind |
|---|------|------|
| 1 | `workspace_context` | read |
| 2 | `engineering_facts` | read |
| 3 | `engineering_memory` | read |
| 4 | `memory_stats` | read |
| 5 | `apply_change` | guarded write |
| 6 | `apply_changes` | transactional write (all-or-nothing, rollback) |
| 7 | `record_memory` | write |
| 8 | `delete_memory` (confirm-gated) | write |
| 9 | `update_identity` | declared-intent write |
| 10 | `sandbox_exec` | policy-gated exec |
| 11 | `sandbox_test` | verified exec |
| 12 | `sandbox_build` | verified exec |
| 13 | `sandbox_status` | read |
| 14 | `impact_analyze` | read |
| 15 | `reindex` | rebuild |
| 16 | `repository_health` | read |
| 17 | `consult` | external call |

Frozen contract: [`docs/MCP_API_V1.md`](docs/MCP_API_V1.md).

## Trust model

Three information classes never blur:

| Class | Source | Trust |
|---|---|---|
| Verified facts | tree-sitter scan by `codebro init` | high — provenance-carrying, validated |
| Identity / decisions | human-authored + deterministic inference | medium-high |
| Agent-recorded memory | `record_memory` calls | low — self-declared confidence |

There is no promotion path from agent memory into the fact store.

## CLI

```
codebro serve --root <path>    # MCP server over stdio
codebro init --root <path>     # scan workspace → .codebro/facts.json
codebro facts diff --root <p>  # changed files + impact vs last index
codebro doctor --root <path>   # diagnose runtime state
codebro list-models            # provider model discovery
codebro consult ...            # ask the consultant from the terminal
codebro auth status            # consultant auth state
```

## Historical note

Versions ≤0.6 were a TUI coding assistant; a later design phase added an
"Adaptive Developer Platform" (intent/preference/recommendation engines).
Both directions are retired and their code was deleted in v1.0
(`docs/LEGACY_RETIREMENT.md`); history remains on the `tui-legacy` branch
and the v0.7 tags.
