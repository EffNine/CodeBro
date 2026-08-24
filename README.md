# CodeBro

**CodeBro is an MCP-based engineering intelligence runtime that provides AI coding agents with repository understanding, verified facts, engineering memory, impact analysis, guarded changes, and reproducible execution evidence.**

AI coding agents are good at reasoning, but they repeatedly rediscover project
structure and forget engineering decisions across sessions. CodeBro solves this
by maintaining verified facts and recorded memory that persist between sessions.

Architecture and module map: [ARCHITECTURE.md](ARCHITECTURE.md).

```
              AI Coding Agents
      OpenCode · Claude Code · Codex · Cursor
                      │
                     MCP
                       ▼
                 ┌───────────┐
                 │  CodeBro  │
                 ├───────────┤
                 │ Facts     │
                 │ Memory    │
                 │ Identity  │
                 │ Impact    │
                 │ Guarded   │
                 │ Changes   │
                 │ Evidence  │
                 │ Consultant│
                 └─────┬─────┘
                       │
               ConsultantProvider
                       │
                       ▼
                 Conductor
              (HTTP + Bearer key)
                       │
               routing / scoring / health
                       │
                       ▼
                Upstream providers
```

**Host agent owns:** reasoning, planning, tool selection, execution strategy, UX.
**CodeBro owns:** project identity, verified project facts, persistent engineering memory, impact analysis, guarded/transactional mutations, sandboxed execution evidence, and consultant queries via Conductor.

CodeBro is **not**:
- a replacement coding agent
- a TUI or chat assistant
- a model provider
- a generic shell / filesystem / Git MCP
- an autonomous agent loop

The frozen v1 tool contract is documented in
[`docs/MCP_API_V1.md`](docs/MCP_API_V1.md); the historical design narrative
lives in [`docs/design/MCP_SERVER.md`](docs/design/MCP_SERVER.md).

## Quick Start

```bash
git clone https://github.com/EffNine/CodeBro.git
cd CodeBro
cargo install --path crates/mcp-server   # binary name: codebro

# From a target project:
codebro init        # scan the workspace into .codebro/facts.json
codebro doctor      # verify the runtime state
opencode mcp add codebro -- "$(which codebro) serve"
```

For a complete Conductor setup guide, see [`docs/CONDUCTOR_HOWTO.md`](docs/CONDUCTOR_HOWTO.md).

## What CodeBro Provides (v1 contract: 17 tools)

| Surface | Tools | Purpose |
|---------|-------|---------|
| Orientation | `workspace_context`, `repository_health` | Project identity, fact counts, workspace diagnostics |
| Verified facts | `engineering_facts`, `reindex`, `impact_analyze` | Relevance-ranked retrieval over the validated fact graph; structural impact with per-edge confidence and evidence |
| Engineering memory | `engineering_memory`, `memory_stats`, `record_memory`, `delete_memory` | Persistent, trust-aware agent-recorded memory (never promoted into facts) |
| Identity | `update_identity` | Declared-intent store: constraints, decisions, roadmap |
| Guarded changes | `apply_change`, `apply_changes` | Single-file guarded mutation; multi-file all-or-nothing transaction with rollback |
| Execution evidence | `sandbox_status`, `sandbox_exec`, `sandbox_test`, `sandbox_build` | Policy-gated commands with full evidence envelopes (git revision, exit code, reproducibility, environment) |
| Consultant | `consult` | Ask Conductor-backed providers for architecture/debug/review opinions |

## Trust Model

Three distinct classes of information — never blurred:

| Class | Source | Trust |
|-------|--------|-------|
| **Verified facts** | `codebro init` (tree-sitter scan) | High — deterministic, validated (0-issue store) |
| **Engineering decisions** | Human-authored identity/constraints | Medium-high — declared intent |
| **Agent-recorded memory** | Agents calling `record_memory` | Low — unverified, self-declared confidence |

Agent-recorded memory is **never** promoted to the verified fact store.

## Installation

Requires a Rust toolchain (see `rust-toolchain.toml`).

```bash
cargo build --release
cargo install --path crates/mcp-server
```

## CLI

```bash
codebro init       # Scan workspace → .codebro/facts.json
codebro doctor     # Diagnostics (exit 0 ok / 1 warn / 2 error)
codebro serve      # MCP server over stdio
codebro facts diff # Diff current repo state vs last index + impact projection
codebro list-models # List models from configured provider
codebro consult    # Ask Conductor a question directly
codebro auth status # Check consultant provider auth
```

## Links

- [Frozen v1 MCP Contract](docs/MCP_API_V1.md)
- [v1.0.0 Release Notes](docs/RELEASE_v1.0.0.md)
- [Conductor Setup HOWTO](docs/CONDUCTOR_HOWTO.md)
- [Architecture Decision Records](docs/ADR/)
- [Changelog](CHANGELOG.md)
- [License](LICENSE)

---

*Current public release: [v1.0.0](https://github.com/EffNine/CodeBro/releases/tag/v1.0.0).*

The former chat TUI is preserved on the `tui-legacy` branch; the development
line is MCP-first only.
