# CodeBro

Engineering context & memory layer for AI coding agents.

CodeBro is a **terminal-native engineering context layer** built with Rust:
persistent, verified project facts plus recorded engineering memory, exposed
to AI coding agents over the **Model Context Protocol (MCP)**. Battle-tested
agent frontends — Claude Code, OpenCode, Codex, Cursor, Goose — connect to
`codebro serve` and get project-wide facts and prior decisions without
re-discovering the codebase or re-learning decisions every session.

> The legacy chat TUI moved to the `tui-legacy` branch
> ([github.com/EffNine/CodeBro/tree/tui-legacy](https://github.com/EffNine/CodeBro/tree/tui-legacy)).
> This branch is the MCP-first engineering runtime.

## Features

### MCP server — engineering context & memory layer

CodeBro is positioned as an **engineering context layer** for AI coding
agents. It answers "where am I, what are this project's verified facts,
and what do we know from past sessions" — so agents stop re-discovering
the codebase and re-learning decisions every session.

- **`workspace_context`** — orient: project identity, workspace root, fact
  store counts. Compact "where am I" answer.
- **`engineering_facts`** — relevance-ranked fact retrieval over the verified fact store
  (symbols, modules, tests, packages, build targets, dependencies). Query
  by name/path fragment, filter by kind/path, returns **fact records with
  locations and provenance** (not raw ids).
- **`engineering_memory`** — resolve recorded decisions/constraints by task
  keywords, with confidence, source and tags so agents can judge
  trustworthiness.
- **`memory_stats`** — entry count, budget, confidence, recency, tag
  distribution — "does engineering memory hold meaningful state?"
- **`record_memory` / `delete_memory`** — guarded memory write path:
  secret-redacted, validation-bounded, persists across sessions
  (anti-amnesia).
- **`apply_change`** *(optional)* — guarded single-file mutation through the
  ChangeEngine (workspace boundary, no blind overwrites, stale-content
  refusal). Available for controlled/autonomous workflows; agents use their
  native editing tools for normal edits.

### Trust model

CodeBro keeps three distinct classes of information and never blurs them:

| Class | Source | Trust |
|---|---|---|
| Verified facts | `codebro init` (tree-sitter scan) | High — deterministic, validated (0-issue store) |
| Engineering decisions | human-authored identity/constraints | Medium-high |
| Agent-recorded memory | agents calling `record_memory` | Low — unverified, self-declared confidence |

Agent-recorded memory is **never** promoted to the verified fact store.
See [`docs/design/MCP_SERVER.md`](docs/design/MCP_SERVER.md) §2.1.

### Tooling

- **`codebro init`** — scan workspace → validated fact store
  (`.codebro/facts.json`): modules, symbols, tests, build targets,
  dependencies from `Cargo.toml` (Rust) or `go.mod` (Go), using
  tree-sitter.
- **`codebro doctor`** — diagnostics: identity/facts/memory/git, scriptable
  exit codes (0 ok / 1 warn / 2 error).
- **`codebro serve`** — the MCP server over stdio.
- **`codebro list-models`** — list models from the configured provider.

## Architecture

> See [Project Structure](#project-structure) for the source layout.

### MCP Architecture (current)

```
              ┌─────────────────┐
              │  Claude Code    │
              │  OpenCode       │
              │  Codex          │
              │  Cursor         │
              └────────┬────────┘
                       │
                       │ MCP (stdio)
                       ▼
              ┌─────────────────┐
              │     codebro     │
              │  `codebro serve`│
              ├─────────────────┤
              │ Engineering     │
              │ Runtime         │
              ├─────────────────┤
              │ project_identity│
              │ fact_store      │
              │ engineering_    │
              │   memory        │
              │ change engine   │
              │ permissions     │
              └─────────────────┘
                       │
                       ▼
                 Repository
```

Control plane (CLI):

```text
codebro init    scan workspace → .codebro/facts.json (modules, symbols,
                tests, build targets, dependencies)
codebro serve   MCP server over stdio (7 tools)
codebro doctor  diagnostics: identity/facts/memory/git, exit 0|1|2
codebro list-models   list models from the configured provider
```

### MCP tools

| Tool | Read/Write | Purpose |
|---|---|---|
| `workspace_context` | read | Orient: identity, root, fact counts |
| `engineering_facts` | read | Relevance-ranked fact retrieval (query/kind/path, records with locations) |
| `engineering_memory` | read | Resolve recorded decisions/constraints by keywords |
| `memory_stats` | read | Memory state: counts, confidence, recency, tags |
| `record_memory` | write | Upsert a memory entry (secret-redacted) |
| `delete_memory` | write | Delete a memory entry by key |
| `apply_change` | write *(optional)* | Guarded single-file mutation via ChangeEngine |

Full design: [`docs/design/MCP_SERVER.md`](docs/design/MCP_SERVER.md).

## Installation

```bash
cargo build --release
cargo install --path .
```

Or clone and build:

```bash
git clone <repo>
cd codebro
cargo build --release
```

## Usage

### Quick start

```bash
# 1. Populate the fact store for a workspace
codebro init --root /path/to/project        # or cd into the project and run `codebro init`

# 2. Check the runtime state (exit code 0 ok / 1 warn / 2 error)
codebro doctor --root /path/to/project

# 3. Serve the MCP server over stdio
codebro serve --root /path/to/project
```

Connect a host agent (e.g. OpenCode):

```bash
opencode mcp add codebro -- \
  /path/to/codebro/target/release/codebro serve --root /path/to/project
```

The server instructions direct agents to consult CodeBro for project facts
and recorded decisions — no prompt hints required. Verified end-to-end with
OpenCode (A/B comparison and auto-detection tests in
[`docs/design/MCP_SERVER.md`](docs/design/MCP_SERVER.md) §9–§10).

## Configuration

The MCP server itself needs no configuration. `codebro list-models` reads an
optional provider config:

```toml
# ~/.codebro/config.toml
provider = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
```

Or environment variables:

```bash
export CODEBRO_API_KEY="your-api-key"
export CODEBRO_BASE_URL="https://api.openai.com/v1"
export CODEBRO_MODEL="gpt-4o"
```

## Example interaction

```text
Agent session starts → calls codebro_workspace_context
  {"workspace_root": "/project", "fact_counts": {"symbols": 10515, ...}}

Agent asks "where is ChangeEngine?" → calls codebro_engineering_facts
  {"query": "ChangeEngine", "kind": "symbol"}
  {"facts": [{"kind": "symbol", "name": "ChangeEngine",
              "path": "src/coding/permissions.rs", "line": 202, ...}]}

Agent learns a decision → calls codebro_record_memory
  {"key": "architecture:change-engine", "value": "...", "confidence": 0.9}
```

## Project Structure

```
codebro/
├── Cargo.toml
├── README.md
├── LICENSE
├── .gitignore
├── src/
│   ├── main.rs              # Entry point
│   ├── error.rs             # Error types and recovery
│   ├── cli/                 # CLI parsing (list-models, serve, init, doctor)
│   ├── mcp/                 # MCP server: 7 tools over stdio
│   │   ├── mod.rs           #   tool definitions + handler
│   │   └── facts.rs         #   semantic fact retrieval engine
│   ├── init/                # Fact-store population (codebro init)
│   ├── doctor/              # Diagnostics (codebro doctor)
│   ├── engineering_facts/   # Canonical facts model (P10.5.0)
│   ├── fact_store/          # Immutable indexed fact store + validation
│   ├── engineering_memory/  # Decision/constraint memory runtime
│   ├── project_identity/    # Project identity runtime
│   ├── coding/              # ChangeEngine: guarded mutation seam
│   │   └── permissions.rs   #   prepare/apply, workspace boundary
│   ├── intelligence/        # Tree-sitter parser platform
│   ├── agent/               # Agent core (used by the runtime)
│   ├── providers/           # LLM providers
│   ├── tools/               # Tool system (filesystem, shell, git, patch)
│   ├── scanner/             # Project scanner
│   ├── dispatcher/          # Tool dispatcher
│   └── config/              # Configuration
└── docs/
    └── design/MCP_SERVER.md # MCP architecture + verified OpenCode tests
```

> The legacy chat TUI (v0.x) was moved to the `tui-legacy` branch.
> The legacy `src/context/` (context builder) and `src/prompt/` (prompt
> assembly) modules were removed in ADR-012. Canonical replacements:
> `src/assembly/` + `src/engineering_context/` and `src/prompt_builder/`
> (`compile_context(&EngineeringContext)`).

## License

MIT

---
