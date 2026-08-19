# CodeBro

Persistent engineering context and memory for AI coding agents, exposed through MCP.

AI coding agents are good at reasoning, but they repeatedly rediscover project
structure and forget engineering decisions across sessions. CodeBro solves this
by maintaining verified facts and recorded memory that persist between sessions.

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
                 │ Guarded   │
                 │ Changes   │
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
**CodeBro owns:** project identity, verified project facts, persistent engineering memory, optional guarded mutations, and consultant queries via Conductor.

CodeBro is **not**:
- a replacement coding agent
- a model provider
- a generic shell / filesystem / Git MCP
- an autonomous agent loop

For the full MCP contract, see [`docs/design/MCP_SERVER.md`](docs/design/MCP_SERVER.md).

## Quick Start

```bash
git clone https://github.com/EffNine/CodeBro.git
cd CodeBro
cargo install --path .

# From a target project:
codebro init
codebro doctor
opencode mcp add codebro -- "$(which codebro) serve"
```

For a complete Conductor setup guide, see [`docs/CONDUCTOR_HOWTO.md`](docs/CONDUCTOR_HOWTO.md).

## What CodeBro Provides

| Surface | Tool | Purpose |
|---------|------|---------|
| Read | `workspace_context` | Orient: project identity, root, fact counts |
| Read | `engineering_facts` | Relevance-ranked fact retrieval (symbols, modules, tests, packages, build targets, dependencies) |
| Read | `engineering_memory` | Resolve recorded decisions/constraints by task keywords |
| Read | `memory_stats` | Memory state: entry count, confidence, recency, tags |
| Write | `record_memory` | Upsert a persistent memory entry (secret-redacted) |
| Write | `delete_memory` | Delete a memory entry by exact key |
| Write | `apply_change` *(optional)* | Guarded single-file mutation via ChangeEngine |
| Read/Write | `consult` | Ask Conductor for opinions (architecture, debugging, code review, planning, research, second opinion) |

## Trust Model

Three distinct classes of information — never blurred:

| Class | Source | Trust |
|-------|--------|-------|
| **Verified facts** | `codebro init` (tree-sitter scan) | High — deterministic, validated (0-issue store) |
| **Engineering decisions** | Human-authored identity/constraints | Medium-high — declared intent |
| **Agent-recorded memory** | Agents calling `record_memory` | Low — unverified, self-declared confidence |

Agent-recorded memory is **never** promoted to the verified fact store.

## Installation

```bash
cargo build --release
cargo install --path .
```

## CLI

```bash
codebro init       # Scan workspace → .codebro/facts.json
codebro doctor     # Diagnostics (exit 0 ok / 1 warn / 2 error)
codebro serve      # MCP server over stdio
codebro list-models # List models from configured provider
codebro consult    # Ask Conductor a question directly
codebro auth status # Check consultant provider auth
```

## Links

- [MCP Server Design](docs/design/MCP_SERVER.md)
- [Conductor Setup HOWTO](docs/CONDUCTOR_HOWTO.md)
- [Architecture Decision Records](docs/ADR/)
- [Changelog](CHANGELOG.md)
- [License](LICENSE)

---

*Current public release: v0.7.0-mcp-rc2 (release candidate).*

The former chat TUI is preserved on the `tui-legacy` branch; the current
`main` branch is MCP-first.
