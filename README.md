# CodeBro

Your AI coding partner in the terminal.

CodeBro is a **terminal-native engineering runtime** built with Rust: persistent
engineering context (facts, memory, project identity) plus guarded code
operations, exposed to AI coding agents over the **Model Context Protocol
(MCP)**. Battle-tested agent frontends — Claude Code, OpenCode, Codex, Cursor,
Goose — connect to `codebro serve` and get project-wide facts and a guarded
mutation path without re-discovering the codebase every session.

It also ships a chat **TUI** — moved to the `tui-legacy` branch
([github.com/EffNine/CodeBro/tree/tui-legacy](https://github.com/EffNine/CodeBro/tree/tui-legacy)).
This branch is the MCP-first engineering runtime.

## Features

### Engineering Runtime (MCP-first)

- **MCP server** (`codebro serve`): exposes the engineering runtime over
  stdio — `workspace_context`, `engineering_facts`, `engineering_memory`,
  `apply_change`, `record_memory`, `delete_memory`
- **Fact store** (`codebro init`): scans the workspace with tree-sitter and
  freezes a validated model (modules, symbols, tests, build targets,
  dependencies from `Cargo.toml`) into `.codebro/facts.json`
- **Diagnostics** (`codebro doctor`): health checks for identity, facts,
  memory and git state with scriptable exit codes (0 ok / 1 warn / 2 error)
- **Engineering memory**: record/update/delete decisions and constraints,
  resolved by task relevance with confidence — secret-redacted before
  storage; persists across sessions (anti-amnesia)
- **Guarded mutations** (`apply_change`): single-file edits through the
  change engine — workspace boundary, no blind overwrites, stale-content
  refusal; the agent is told never to bypass a rejected guard
- **Auto-detection**: server instructions direct agents to consult CodeBro
  for project-wide facts before grepping — no prompt hints required
  (verified end-to-end with OpenCode, see `docs/design/MCP_SERVER.md` §10)

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
codebro serve   MCP server over stdio (6 tools)
codebro doctor  diagnostics: identity/facts/memory/git, exit 0|1|2
codebro list-models   list models from the configured provider
```

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

## Configuration

Create `~/.codebro/config.toml`:

```toml
provider = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
```

Or set environment variables (override config file):

```bash
export CODEBRO_API_KEY="your-api-key"
export CODEBRO_BASE_URL="https://api.openai.com/v1"
export CODEBRO_MODEL="gpt-4o"
```

## Usage

### Engineering runtime (recommended)

```bash
# 1. Populate the fact store for a workspace
codebro init --root /path/to/project        # or cd into the project and run `codebro init`

# 2. Check the runtime state (exit code 0 ok / 1 warn / 2 error)
codebro doctor --root /path/to/project

# 3. Serve the engineering runtime over MCP stdio
codebro serve --root /path/to/project
```

Connect a host agent (e.g. OpenCode):

```bash
opencode mcp add codebro -- \
  /path/to/codebro/target/release/codebro serve --root /path/to/project
```

The agent then auto-consults CodeBro for project facts (symbols, modules,
tests, dependencies), recorded decisions, and guarded edits — no prompt
hints required. See [`docs/design/MCP_SERVER.md`](docs/design/MCP_SERVER.md) §8.

## Supported Providers

- OpenAI (GPT-4, GPT-4o, etc.)
- OpenRouter
- DeepSeek
- Ollama (local models)
- LM Studio (local models)

## Examples

```
User: Explain this repository
AI: This is a Rust project using actix-web...

User: Add a login page
AI: PLAN:
1. Inspect project structure
2. Locate frontend files
3. Modify components
4. Run tests

User: Run the tests
AI: Running cargo test...
   test result: ok. 15 passed
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
│   ├── mcp/                 # MCP server: 6 tools over stdio
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

## CodeBro v0.4 Reliability Layer

> Historical release notes (v0.4 and earlier). The current architecture is
> the MCP-first engineering runtime described above; the chat TUI moved to
> the `tui-legacy` branch.

CodeBro v0.4 introduces a comprehensive Reliability Layer to make CodeBro a more stable long-running adaptive coding agent.

### Reliability Architecture

```
codebro/
├── Cargo.toml
├── README.md
├── LICENSE
├── .gitignore
└── src/
    ├── main.rs
    ├── error.rs
    ├── cli/
    │   └── mod.rs
    ├── tui/
    │   ├── mod.rs
    │   ├── app.rs
    │   ├── events.rs
    │   └── ui.rs
    ├── agent/
    │   ├── mod.rs
    │   ├── agent.rs         # Agent orchestration (with reliability)
    │   ├── planner.rs       # Memory-aware planning
    │   ├── memory.rs        # Session memory
    │   ├── memory_manager.rs # Memory consolidation engine
    │   ├── skill.rs         # Skill lifecycle system
    │   ├── trace.rs         # Agent operation trace
    │   ├── permissions.rs   # Permission safety layer
    │   └── workspace.rs     # Workspace awareness
    ├── providers/           # LLM providers
    │   ├── mod.rs
    │   ├── provider.rs
    │   └── openai.rs
    ├── tools/               # Tool system
    │   ├── mod.rs
    │   ├── filesystem.rs
    │   ├── shell.rs         # Shell with timeout & history
    │   ├── git.rs
    │   └── patch.rs
    ├── indexer/
    │   ├── mod.rs
    │   └── scanner.rs
    ├── scanner/
    │   ├── mod.rs
    │   └── project.rs
    ├── dispatcher/
    │   ├── mod.rs
    │   └── registry.rs
    └── config/
        └── mod.rs
```

> The legacy `src/context/` and `src/prompt/` modules were removed in ADR-012.

### Memory Lifecycle

CodeBro v0.4 includes a Memory Consolidation Engine that prevents unbounded memory growth:

**Memory Scoring:**
Each memory entry is scored based on:
- **Importance** (30%): User-assigned or system-derived importance
- **Confidence** (25%): How reliable the memory entry is
- **Usage frequency** (20%): How often the memory is referenced
- **Recency** (25%): How recently the memory was used

**Consolidation Operations:**
1. **Duplicate Detection**: Identifies and removes near-duplicate memories
2. **Similar Memory Merging**: Combines related memories into consolidated entries
3. **Outdated Memory Cleanup**: Removes memories older than 90 days
4. **Low-Value Memory Removal**: Removes low-scoring, rarely-used memories

**Example:**
Before consolidation:
- "User ran cargo test"
- "cargo test passed"

After consolidation:
- "Project validates changes using cargo test"

### Skill Lifecycle

Skills now follow a structured lifecycle:

```
Draft → Testing → Trusted → Deprecated
```

**Lifecycle Rules:**
- New skills start as `Draft`
- After 3+ successful uses with 70%+ success rate → `Testing`
- After 5+ uses with 80%+ success rate → `Trusted`
- After repeated failures or low success rate → `Deprecated`

**Skill Validation:**
Before applying a skill, CodeBro checks:
- Project compatibility (language/framework match)
- Previous success rate (minimum 30% confidence)
- Skill status (deprecated skills are not applied)

**Skill Conflict Resolution:**
When multiple skills match, CodeBro ranks them by:
1. Project-specific skills first
2. Most recent usage
3. Highest success rate
4. Highest confidence

### Permission Model

CodeBro v0.4 includes a Permission Safety Layer:

**Permission Levels:**
- **Allow**: Automatically permitted (e.g., `read_file`, `list_files`, `git_status`, `git_diff`)
- **Ask**: Requires user confirmation (e.g., `write_file`, `edit_file`)
- **Deny**: Explicitly blocked (e.g., `delete_file`, `rm`, `git push`)

**Dangerous Patterns:**
The permission system automatically flags dangerous operations:
- `rm -rf`, `rm -` commands
- `git push`, `git reset --hard`, `git clean`
- `shutdown`, `reboot`, `format`
- `chmod -R`

### Agent Operation Trace

Every agent operation is recorded in `.codebro/traces/`:

```json
{
  "task_id": "task-1",
  "timestamp": "2024-01-01T00:00:00Z",
  "user_request": "Add API endpoint",
  "plan_summary": "Read files, patch, test",
  "tools_executed": ["read_file", "patch_file", "cargo_test"],
  "files_changed": ["src/main.rs"],
  "commands_executed": ["cargo test"],
  "result": "success",
  "lesson_learned": null,
  "memory_influence": ["Project uses cargo test"],
  "skill_used": "rust_build"
}
```

**Trace Guidelines:**
- Only operational summaries are stored
- Private chain-of-thought is NOT stored
- Traces are persisted as JSON files in `.codebro/traces/`

### Shell Session Improvements

The shell tool now includes:
- **Command timeout**: Configurable timeout (default 300s) to prevent hanging commands
- **Working directory tracking**: Commands execute in specified directories
- **Environment tracking**: Custom environment variables per command
- **Command history**: All commands logged to `.codebro/shell_history.json`

### Workspace Awareness

CodeBro tracks project context in `.codebro/workspace.json`:

```json
{
  "root": "/path/to/project",
  "language": "rust",
  "framework": "actix-web",
  "build_system": "cargo",
  "active_files": ["src/main.rs", "src/lib.rs"],
  "recent_files": ["src/main.rs", "Cargo.toml"],
  "recent_commands": ["cargo test", "cargo build"],
  "updated_at": "2024-01-01T00:00:00Z"
}
```

### Agent Workflow (v0.4)

```
User Request
    ↓
Retrieve relevant memories (Memory Consolidation Engine)
    ↓
Retrieve matching skills (Skill Lifecycle + Validation)
    ↓
Retrieve previous plans (Plan Memory)
    ↓
Create plan (with reasoning, memory influence, skill used)
    ↓
Check permissions (Permission Manager)
    ↓
Execute tools (with timeout & tracking)
    ↓
Evaluate result
    ↓
Store lesson (Trace + Reflection)
    ↓
Consolidate memory (dedup, merge, cleanup)
    ↓
Update workspace context
    ↓
Record operation trace
```

## Code Intelligence Architecture (v0.5)

CodeBro v0.5 introduces a comprehensive Code Intelligence Layer that transforms the agent from file-based understanding into code-aware understanding.

### Overview

```
User Question
    ↓
Semantic Search (intelligence/search)
    ↓
Symbol Lookup (intelligence/index)
    ↓
Dependency Graph (intelligence/graph)
    ↓
Relevant Context (intelligence/context)
    ↓
Prompt Builder
```

### Tree-sitter Integration

CodeBro uses Tree-sitter for accurate code parsing across multiple languages:

- **Rust**: `tree-sitter-rust` - parses functions, structs, enums, traits, impls, macros, modules, imports
- **Python**: `tree-sitter-python` - parses functions, classes, imports
- **JavaScript**: `tree-sitter-javascript` - parses functions, classes, methods, imports, exports
- **TypeScript**: `tree-sitter-typescript` - parses functions, classes, interfaces, type aliases, methods, imports, exports
- **Go**: `tree-sitter-go` - parses functions, types, structs, interfaces, methods, imports

All parsing is done through structured `ParseResult` containing `ParsedSymbol` entries with name, kind, location, signature, and doc comments.

### Symbol Index

Symbols are stored in an SQLite database at `.codebro/code_index.db` with full indexing support:

- **Symbol table**: name, kind, language, file, line range, parent, visibility, signature, doc comment
- **Relationship table**: tracks contains, calls, imports, references between symbols
- **Incremental updates**: only re-index changed files
- **File removal**: automatically removes symbols for deleted files

### Semantic Code Search

The semantic search engine ranks results by relevance:

1. **Exact name match** (score: 3.0)
2. **Prefix match** (score: 2.0)
3. **Partial name match** (score: 1.5)
4. **Signature match** (score: 1.0)
5. **Doc comment match** (score: 0.8)
6. **File relevance** (score: 0.5)

Example queries:
- "where is authentication handled?" → finds AuthService, JWT middleware, login handler
- "where database connection created?" → finds database module, connection pool, config

### Dependency Graph

The dependency graph tracks relationships between files and symbols:

- **Nodes**: files with their symbols
- **Edges**: import/dependency relationships
- **Transitive dependencies**: find all files a file depends on
- **Transitive dependents**: find all files that depend on a file
- **Path finding**: find dependency paths between files

### Intelligent Context Builder

Upgraded context builder that selects the most relevant code context:

1. Semantic search for relevant symbols
2. Symbol lookup with confidence scoring
3. Dependency graph analysis
4. Relevant context assembly
5. Prompt building with small, focused context

Prefers small relevant context over large file dumps.

### LSP Foundation

CodeBro includes an LSP integration architecture with support for:

- **Diagnostics**: code error reporting
- **Go to definition**: navigate to symbol definitions
- **References**: find all references to a symbol
- **Rename**: rename symbols across the project

The LSP foundation provides interfaces for future full implementation.

### Agent Reasoning Upgrade

The planner now uses code intelligence before modifying code:

1. **Semantic Search**: finds relevant symbols related to the user's request
2. **Symbol Lookup**: retrieves detailed symbol information
3. **Dependency Analysis**: analyzes dependencies and relationships
4. **Context Assembly**: assembles the most relevant context
5. **Plan Generation**: creates an informed implementation plan

Example: "Add caching" → finds Cache interface, Database layer, Existing config pattern → Plan: extend cache abstraction, implement provider, add tests

### Intelligence Memory

> **Note:** The `IntelligenceMemory` store (`intelligence/memory/`, persisted to
> `~/.codebro/project_memory.json`) was removed in ADR-012. Persistent project
> knowledge is now owned by `project_identity` (`.codebro/project_identity.json`)
> and task-relevant memory by `engineering_memory`
> (`.codebro/engineering_memory.json`).

The original v0.5 design stored learned codebase knowledge in
`.codebro/project_memory.json`:

- **Important symbols**: core functions, classes, structs, traits
- **Architecture patterns**: discovered design patterns
- **Conventions**: coding conventions and patterns
- **Discovered relationships**: symbol dependencies and relationships
- **Project structure**: modules, layers, entry points, public API

### Example Intelligence Workflow

```
User: "Add caching to the authentication flow"

CodeBro:
1. Searches for "authentication" → finds AuthService, login_handler, JWT middleware
2. Looks up dependencies → finds database layer, config module
3. Checks for existing cache patterns → finds Cache trait, Redis provider
4. Assembles context → AuthService code, database connection, cache trait
5. Generates plan:
   - Extend Cache trait with TTL support
   - Implement Redis cache provider
   - Add cache configuration
   - Update AuthService to use cache
   - Add cache tests
```

### Future LSP Support

Planned LSP features for future versions:
- Full diagnostics with error severity
- Go to definition with jump-to-file support
- Find all references across the project
- Rename symbol with automatic updates
- Hover information with type details
- Auto-completion with context-aware suggestions

## Multi-Agent Architecture (v0.6)

CodeBro v0.6 transforms the project from a single-agent coding assistant into an autonomous multi-agent execution system.

### Architecture

```
User Request
    ↓
Main Agent Orchestrator
    ↓
+----------------+
|                |
v                v
Planner Agent   Task Router
    ↓
Subagents (Research, Planning, Coding, Testing, Review)
    ↓
Tool Execution
    ↓
Verification
    ↓
Reflection
    ↓
Memory + Skill Update
```

### Subagent Framework

- **Research Agent**: Understands codebase, gathers information
- **Planning Agent**: Creates implementation plans
- **Coding Agent**: Modifies code
- **Testing Agent**: Validates changes
- **Review Agent**: Reviews implementation quality

### Key Components

- **Task Router**: Analyzes task complexity (simple/moderate/complex) and routes to appropriate agents
- **Task Graph Engine**: Represents tasks as a DAG with dependencies and parallel execution
- **Experience Replay**: Stores successful workflows and reuses patterns
- **Smart Tool Router**: Selects the right tools based on task intent
