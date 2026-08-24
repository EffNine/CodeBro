# CodeBro Architecture

**CodeBro is an MCP-based engineering intelligence runtime that provides AI coding agents with repository understanding, verified facts, engineering memory, impact analysis, and safe execution.**

It is not a chat UI, not an agent loop, and not a model provider. The host
agent (OpenCode, Claude Code, Codex, Cursor, ...) owns planning, tool
selection and conversation; CodeBro owns engineering truth and safe execution,
exposed over MCP stdio (`codebro serve`).

```
AI Coding Agent (OpenCode / Claude Code / Cursor)
        │  MCP over stdio (rmcp)
        ▼
CodeBroMcpServer ──────────── src/mcp/  (16 tools)
        │
        ▼
Engineering Runtime
        ├─ project_identity/   declared intent (.codebro/project_identity.json)
        ├─ engineering_facts/  canonical fact model (symbols, modules, tests, deps…)
        ├─ fact_store/         immutable indexed store (validated, mtime-cached)
        ├─ impact/             AST-verified relationship graph + BFS traversal
        ├─ engineering_memory/ persistent memory (bounded resolution, trust tiers)
        ├─ memory_runtime/     generic memory engine underneath engineering_memory
        └─ init/               tree-sitter indexing pipeline (Rust, Go)
        │
        ▼
Core Services
        ├─ coding/             ChangeEngine — the single guarded mutation seam
        ├─ sandbox/            policy-gated execution (Local PTY | OpenSandbox)
        ├─ consultant/         Conductor-backed consult capability
        ├─ doctor/             workspace health diagnostics
        ├─ tools/              shell exec + patch/change machinery + shared types
        ├─ providers/          OpenAI-compatible model discovery
        └─ config/, credentials/, error, persistence, provenance, cancellation
```

## Dependency direction

The rule is strict and one-way:

```
MCP  →  Engineering Runtime  →  Core Services
```

No live module imports from `src/legacy/`. The legacy tree (pre-MCP TUI
agent stack and Adaptive Platform subsystems) is compiled only under
`#[cfg(test)]` so its regression suite keeps passing during migration; it has
no entry point from `main` and must not gain new functionality.

## The 16 MCP tools

| # | Tool | Kind |
|---|------|------|
| 1 | `workspace_context` | read |
| 2 | `engineering_facts` | read |
| 3 | `engineering_memory` | read |
| 4 | `memory_stats` | read |
| 5 | `apply_change` | guarded write |
| 6 | `record_memory` | write |
| 7 | `delete_memory` (confirm-gated) | write |
| 8 | `update_identity` | declared-intent write |
| 9 | `sandbox_exec` | policy-gated exec |
| 10 | `sandbox_test` | verified exec |
| 11 | `sandbox_build` | verified exec |
| 12 | `sandbox_status` | read |
| 13 | `impact_analyze` | read |
| 14 | `reindex` | rebuild |
| 15 | `repository_health` | read |
| 16 | `consult` | external call |

Full contracts: [`docs/design/MCP_SERVER.md`](docs/design/MCP_SERVER.md).

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
codebro doctor --root <path>   # diagnose runtime state
codebro list-models            # provider model discovery
codebro consult ...            # ask the consultant from the terminal
codebro auth status            # consultant auth state
```

## Historical note

Versions ≤0.6 were a TUI coding assistant; a later design phase added an
"Adaptive Developer Platform" (intent/preference/recommendation engines).
Both directions are retired. Their code is preserved under `src/legacy/`
purely as a regression suite and is scheduled for deletion once the live
runtime's own coverage is sufficient.
