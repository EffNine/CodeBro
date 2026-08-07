# CodeBro Architecture Manifest v1.0

**Document:** `docs/architecture/architecture_manifest_v1.md`
**Version:** 1.0.0
**Effective:** P0.75 Engineering Baseline
**Status:** Frozen — all changes require an ADR

---

## 1. Purpose

This manifest defines the immutable architecture of CodeBro. It documents module boundaries, data flow, and architectural contracts that every future phase must respect. Any change to these boundaries requires an approved Architecture Decision Record (ADR).

**Core architectural principle:** Data flows in one direction — from user input, through tool execution and LLM synthesis, back to the user. No module may pull data from a downstream module.

---

## 2. Folder Layout

```
codebro/
├── Cargo.toml
├── README.md
├── LICENSE
├── .gitignore
│
├── src/
│   ├── main.rs                    # Entry point: tracing init → cli::run()
│   ├── error.rs                   # CodeBroError enum (thiserror)
│   │
│   ├── cli/                       # CLI argument parsing
│   │   └── mod.rs                 # clap definitions, model resolution
│   │
│   ├── config/                    # Configuration system
│   │   └── mod.rs                 # Config struct, load/persist, env overrides
│   │
│   ├── session/                   # Session persistence
│   │   └── mod.rs                 # Session, SessionStore, SessionTracker
│   │
│   ├── metrics/                   # Metrics tracking
│   │   └── mod.rs                 # MetricsRegistry, TaskMetrics
│   │
│   ├── tui/                       # Terminal UI (ratatui + crossterm)
│   │   ├── mod.rs
│   │   ├── app.rs                 # TuiApp state machine
│   │   ├── ui.rs                  # Render loop, event dispatch, layout
│   │   ├── dashboard.rs           # AgentStatusMonitor, panels, model picker
│   │   ├── events.rs              # AppEvent enum, keyboard shortcuts
│   │   ├── animation.rs           # Spinner/progress animation
│   │   ├── markdown.rs            # pulldown-cmark → ratatui Lines
│   │   ├── tool_parser.rs         # LLM tool-call string parser
│   │   └── diff_view.rs           # Diff rendering widget
│   │
│   ├── agent/                     # Agent orchestration
│   │   ├── mod.rs
│   │   ├── coordinator.rs         # AgentCoordinator: spawns/manages subagents
│   │   ├── planner.rs             # Plan struct, memory-aware planning
│   │   ├── router.rs              # TaskRouter: complexity → agent selection
│   │   ├── task_graph.rs          # DAG of tasks with status
│   │   ├── events.rs              # AgentEvent enum, EventBus
│   │   ├── status.rs              # AgentStatus enum, AgentStatusMonitor
│   │   ├── communication/         # Agent message bus
│   │   │   └── mod.rs
│   │   ├── subagent/              # Subagent implementations
│   │   │   ├── mod.rs
│   │   │   ├── trait_agent.rs     # SubAgent trait
│   │   │   ├── research.rs        # ResearchAgent (analysis-only)
│   │   │   ├── planning.rs        # PlanningAgent (analysis-only)
│   │   │   ├── coding.rs          # CodingAgent (analysis-only)
│   │   │   ├── testing.rs         # TestingAgent (analysis-only)
│   │   │   └── review.rs          # ReviewAgent (analysis-only)
│   │   ├── memory.rs              # Memory: short-term, project, global
│   │   ├── memory_manager.rs      # MemoryConsolidationEngine
│   │   ├── skill.rs               # SkillManager: lifecycle, confidence
│   │   ├── plan_memory.rs         # PlanMemoryStore
│   │   ├── permissions.rs         # PermissionManager
│   │   ├── workspace.rs           # WorkspaceManager
│   │   ├── trace.rs               # OperationTrace, TraceStore
│   │   ├── reflection.rs          # ReflectionEngine
│   │   ├── recovery.rs            # RecoveryEngine
│   │   ├── decision.rs            # DecisionEngine
│   │   ├── experience.rs          # ExperienceReplay
│   │   ├── performance.rs         # PerformanceLogger
│   │   └── resources.rs           # ResourceManager
│   │
│   ├── tools/                     # Tool execution system
│   │   ├── mod.rs                 # Tool trait + re-exports
│   │   ├── executor.rs            # run_tool_pipeline() — production pipeline
│   │   ├── router.rs              # SmartToolRouter — tool selection
│   │   ├── filesystem.rs          # ListFiles, ReadFile, CreateFile, EditFile
│   │   ├── shell.rs               # RunCommand — timeout, history, redaction
│   │   ├── git.rs                 # GitStatus, GitDiff
│   │   ├── patch.rs               # PatchEngine — diff computation, apply
│   │   └── change.rs              # ChangePlan — propose + approve workflow
│   │
│   ├── providers/                 # LLM provider abstraction
│   │   ├── mod.rs
│   │   ├── provider.rs            # Provider trait
│   │   ├── openai.rs              # OpenAI-compatible provider
│   │   └── models.rs              # Model discovery
│   │
│   ├── intelligence/              # Code intelligence layer (P4)
│   │   ├── mod.rs
│   │   ├── index/                 # Symbol indexing (Tree-sitter → SQLite)
│   │   │   ├── mod.rs
│   │   │   ├── indexer.rs
│   │   │   ├── database.rs
│   │   │   └── symbol.rs
│   │   ├── search/                # Semantic search
│   │   │   ├── mod.rs
│   │   │   └── semantic.rs
│   │   ├── graph/                 # Dependency graph
│   │   │   ├── mod.rs
│   │   │   └── dependency.rs
│   │   ├── parser/                # Tree-sitter parsing
│   │   │   ├── mod.rs
│   │   │   ├── tree_sitter.rs
│   │   │   └── languages.rs
│   │   ├── lsp/                   # LSP foundation (interface stubs)
│   │   │   ├── mod.rs
│   │   │   └── foundation.rs
│   │   ├── context/               # Intelligent context builder
│   │   │   ├── mod.rs
│   │   │   └── builder.rs
│   │   ├── reasoning/             # Reasoning engine
│   │   │   ├── mod.rs
│   │   │   └── engine.rs
│   │   ├── memory/                # Intelligence memory
│   │   │   ├── mod.rs
│   │   │   └── intelligence.rs
│   │   └── diagnostics.rs         # Platform health monitoring (P4)
│   │
│   ├── context/                   # Legacy context builder (token-budget)
│   │   ├── mod.rs
│   │   └── builder.rs
│   ├── prompt/                    # Legacy prompt assembly
│   │   ├── mod.rs
│   │   └── builder.rs
│   ├── indexer/                   # Legacy repo indexer
│   │   ├── mod.rs
│   │   └── scanner.rs
│   ├── scanner/                   # Project scanner
│   │   ├── mod.rs
│   │   └── project.rs
│   ├── dispatcher/                # Legacy tool registry
│   │   ├── mod.rs
│   │   └── registry.rs
│   └── tests.rs                   # Integration tests
│
├── docs/                          # Engineering governance
│   ├── SOP/                       # Standard Operating Procedures
│   ├── RFC/                       # Request for Comments
│   ├── ADR/                       # Architecture Decision Records
│   ├── reports/                   # Phase reports
│   └── roadmap/                   # Development roadmap
│
└── .codebro/                      # Runtime data (gitignored)
    ├── config.toml                # User configuration
    ├── memory.json                # Session/project/global memory
    ├── sessions/                  # Persistent session files
    ├── traces/                    # Operation traces
    ├── code_index.db              # SQLite symbol index
    └── workspace.json             # Workspace metadata
```

---

## 3. Module Boundaries

### 3.1 Hard Boundaries (No Cross-Cutting Exceptions)

| Boundary | Rule | Rationale |
|----------|------|-----------|
| `tui/` → `agent/` | TUI may emit `AgentEvent` but may not call agent logic directly | Separation of concerns; agent logic is testable without TUI |
| `agent/` → `tools/` | Agents may not call tools directly; all tool execution goes through `tools::executor` | Single execution path; consistent event emission |
| `tools/` → `providers/` | Tools may not call LLM providers | Tools are synchronous; providers are async |
| `providers/` → `tools/` | Providers may not call tools | Providers are LLM-only |
| `intelligence/` → `tools/` | Intelligence layer may not execute tools | Intelligence is read-only analysis |
| `config/` → `agent/` | Config may not depend on agent | Config is loaded before agent initialization |
| `session/` → `agent/` | Session tracker does not depend on agent events directly; it receives events via `AgentEvent` clone | Loose coupling via event cloning |

### 3.2 Permitted Data Flow

```
User Input
    ↓
cli/ (parse)
    ↓
tui/ (display + capture)
    ↓
agent/coordinator/ (orchestrate)
    ↓
tools/executor/ (execute tools, produce context)
    ↓
providers/ (LLM call, stream response)
    ↓
tui/ (display response)
    ↓
session/ (persist)
```

**Reverse flow is prohibited.** The TUI sends events *out*; it does not receive data *in* from downstream modules except through the event channel.

---

## 4. Provider Abstraction

### 4.1 Trait Contract

```rust
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn base_url(&self) -> &str;
    fn model(&self) -> &str;
    fn api_key(&self) -> Option<&str>;
    fn send_message(&self, message: &str) -> Pin<Box<dyn Future<Output = Result<String>> + Send + '_>>;
    fn stream_response(&self, message: &str) -> Pin<Box<dyn Future<Output = Result<UnboundedReceiver<String>>> + Send + '_>>;
}
```

### 4.2 Rules

1. **Only one provider is active per session.** The provider is selected from `Config`.
2. **The `Provider` trait is the sole interface to LLM communication.** Direct `reqwest` calls from `tui/` or `agent/` are prohibited.
3. **Streaming must use `stream_response()`.** The `send_message()` path is for non-streaming fallback only.
4. **Provider errors must be wrapped in `CodeBroError::Provider`.** Raw `reqwest` errors must not escape the provider module.
5. **API keys must never leave the provider module.** They are passed in, never returned.

### 4.3 Current Implementation

| Provider | Status | Notes |
|----------|--------|-------|
| `OpenAiProvider` | Active | OpenAI-compatible endpoint (OpenAI, OpenRouter, DeepSeek, Ollama, LM Studio) |
| Others | Stub | `Provider` trait defined; implementations to follow ADR |

---

## 5. Tool Abstraction

### 5.1 Trait Contract

```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn execute(&self, args: &str) -> Result<String>;
}
```

### 5.2 Rules

1. **All tool execution goes through `tools::executor::run_tool_pipeline()`.** Direct tool calls from `tui/` or `agent/` are prohibited except for the legacy `execute_tool_call()` in `tui/ui.rs` (deprecated path, must be removed).
2. **Tool arguments are strings.** Structured argument parsing is the tool's responsibility.
3. **Tool output is capped at `MAX_TOOL_OUTPUT` (32 KB).** No tool may return more than this without truncation.
4. **Secrets are redacted before output leaves the tool.** The `redact_secrets()` function in `shell.rs` is the model.
5. **Tools are synchronous.** Async tools are not permitted; use `tokio::task::spawn_blocking` if needed.
6. **Tools must not modify global state.** Each tool call is stateless except for explicit file writes (which go through `ChangePlan`).

### 5.3 Current Tool Inventory

| Tool | Name | Permission | Description |
|------|------|------------|-------------|
| `ListFiles` | `list_files` | Allow | List files in a directory |
| `ReadFile` | `read_file` | Allow | Read file contents |
| `CreateFile` | `create_file` | Ask | Create a new file |
| `EditFile` | `edit_file` | Ask | Edit file by text replacement |
| `RunCommand` | `run_command` | Ask | Execute shell command |
| `GitStatus` | `git_status` | Allow | Show git status |
| `GitDiff` | `git_diff` | Allow | Show git diff |

---

## 6. Event System

### 6.1 Event Types

| Layer | Event Type | Direction | Purpose |
|-------|-----------|-----------|---------|
| TUI | `AppEvent` | Worker → UI | Keyboard, paste, resize, model fetch, agent events |
| Agent | `AgentEvent` | Agent → UI/Session | Agent lifecycle, tool execution, memory changes |
| Dashboard | `LogEntry` | Internal | Activity log entries |

### 6.2 Rules

1. **All cross-module communication goes through channels.** No shared mutable state between TUI and agent threads except through `AgentEvent` clones.
2. **`AgentEvent` is the only event type that crosses the agent/TUI boundary.** The TUI listens to `AppEvent::AgentEvent(AgentEvent)`.
3. **Event variants are immutable once created.** Use `clone()` to share; never mutate shared events.
4. **No event variant may contain a `String` longer than 10,000 characters.** Long data goes in the dashboard; events carry summaries.
5. **Event ordering is preserved within a single channel.** Do not rely on ordering across channels.

### 6.3 Event Flow

```
User presses key
    ↓
events.rs: AppEvent::Input(key)
    ↓
ui.rs: handle_key() → handle_command() / run_chat_pipeline()
    ↓
run_chat_pipeline() spawns tokio task
    ↓
AgentEvent emitted → AppEvent::AgentEvent(AgentEvent)
    ↓
ui.rs: handle_event() → dashboard.handle_event() + app.handle_agent_event()
    ↓
render()
```

---

## 7. Memory Architecture

### 7.1 Three-Tier Design

```
┌─────────────────────────────────────────────────┐
│  Short-term Memory (in-memory, max 100 entries) │
│  • Recent conversation entries                   │
│  • Auto-pruned when limit exceeded               │
│  • Fast access for current session               │
├─────────────────────────────────────────────────┤
│  Project Memory (.codebro/memory.json)           │
│  • Project summary                               │
│  • Recent files, commands, plans                 │
│  • Tasks, decisions, preferences                 │
│  • Persisted per-project                         │
├─────────────────────────────────────────────────┤
│  Global Memory (.codebro/memory.json)            │
│  • Successful solutions                          │
│  • Lessons learned                               │
│  • Reflection history                            │
│  • Cross-project knowledge                       │
│  • Consolidated by MemoryConsolidationEngine     │
└─────────────────────────────────────────────────┘
```

### 7.2 Rules

1. **Memory is JSON-serialized to `~/.codebro/memory.json`.** No other format is permitted.
2. **The `MemoryConsolidationEngine` runs after every task.** It deduplicates, merges similar entries, and removes outdated/low-value memories.
3. **Short-term memory is bounded at 100 entries.** Oldest entries are dropped first.
4. **Memory search is keyword-based.** Embedding-based search requires a new ADR.
5. **Memory is read-only during tool execution.** Memory updates happen only after task completion.

---

## 8. Session Architecture

### 8.1 Session Lifecycle

```
Session created (new UUID)
    ↓
SessionTracker.start_session(task)
    ↓
Agent events recorded → Session.timeline populated
    ↓
SessionTracker.end_session()
    ↓
Session saved to ~/.codebro/sessions/<id>.json
```

### 8.2 Rules

1. **Each session has a unique UUID.** Sessions are never reused.
2. **Sessions are persisted as pretty-printed JSON.** Format must be human-readable.
3. **Session files are named by UUID:** `<session_id>.json`.
4. **The current session is tracked in memory by `SessionTracker`.** It is not reloaded on every event.
5. **Sessions are auto-saved on every event.** No batch save — every event triggers a write.
6. **Session replay reads the JSON file and reconstructs the timeline.** No in-memory state is needed for replay.

---

## 9. Configuration Architecture

### 9.1 Config Sources (Priority Order)

1. **Environment variables** (`CODEBRO_API_KEY`, `CODEBRO_BASE_URL`, `CODEBRO_MODEL`) — highest priority
2. **Config file** (`~/.codebro/config.toml`) — persisted across sessions
3. **Defaults** — built into the binary

### 9.2 Config Schema

```toml
provider = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
# api_key is stored in environment, not in config file
```

### 9.3 Rules

1. **Config is loaded once at startup.** No runtime config reload.
2. **Config changes require a restart.** Hot-reload is not supported.
3. **API keys are never stored in the config file.** They are passed via environment variables only.
4. **The `model` field may be empty** to trigger auto-detection on first run.
5. **Config validation happens at load time.** Invalid config prevents startup.

---

## 10. TUI Architecture

### 10.1 Layout Model

```
┌─────────────────────────────────────────────────────┐
│ TITLE BAR    (CODEBRO | WS: <name> | Model: <m>    │
│            | Tools: <status> | <spinner>)           │
├─────────────────────────────────────────────────────┤
│ CONVERSATION (scrollable, auto-scrolls on new msg)  │
│                                                     │
│  ─── USER ─────────────────────────────────────     │
│  <user message>                                     │
│                                                     │
│  ─── AI ─────────────────────────────────────       │
│  <ai response (markdown rendered)>                  │
│                                                     │
├─────────────────────────────────────────────────────┤
│ AGENTS (toggleable, shows agent status + progress)  │
├─────────────────────────────────────────────────────┤
│ ACTIVITY LOG (timestamped, color-coded)             │
├─────────────────────────────────────────────────────┤
│ TASK GRAPH (toggleable, shows DAG)                  │
├─────────────────────────────────────────────────────┤
│ METRICS (toggleable, tokens/cost/time)              │
├─────────────────────────────────────────────────────┤
│ COORDINATION (toggleable, agent messages)           │
├─────────────────────────────────────────────────────┤
│ SHORTCUTS BAR  (Ctrl+A Agents | Ctrl+G Graph | ...) │
├─────────────────────────────────────────────────────┤
│ COMMAND PALETTE (toggleable, fuzzy search)          │
├─────────────────────────────────────────────────────┤
│ INPUT  (> user input, multi-line, history nav)      │
└─────────────────────────────────────────────────────┘
```

### 10.2 Layout Engine

- **Dynamic height calculation:** Optional panels compete for space; conversation gets priority.
- **Minimum conversation height:** 4 lines at all times.
- **Panel collapse:** Panels with zero height are not rendered.
- **Resize handling:** On terminal resize, conversation scroll is reset to bottom.

### 10.3 Rules

1. **The TUI is stateless except for `TuiApp`.** No global state.
2. **All rendering goes through `Frame`.** No direct terminal writes from business logic.
3. **The event loop runs at 50ms intervals.** Animations tick within this loop.
4. **The input area is always visible and always has focus.** No focus management needed.
5. **Keyboard shortcuts are checked before key dispatch.** Shortcuts bypass normal input handling.

---

## 11. Intelligence Architecture

### 11.1 Layer Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    Intelligence Platform (P4)                    │
│                     read-only code understanding                 │
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │   Parser     │  │   Indexer    │  │  Symbol Database     │  │
│  │  (tree-sitter)│  │  (SQLite)    │  │  (name, kind, file,  │  │
│  │              │  │              │  │   line, parent,       │  │
│  │              │  │              │  │   visibility, sig,    │  │
│  │              │  │              │  │   doc_comment)        │  │
│  └──────┬───────┘  └──────┬───────┘  └──────────┬───────────┘  │
│         │                 │                      │              │
│  ┌──────▼───────┐  ┌──────▼───────┐  ┌──────────▼───────────┐  │
│  │  Semantic    │  │  Dependency  │  │  Intelligence        │  │
│  │  Search      │  │  Graph       │  │  Memory              │  │
│  │  (keyword)   │  │  (files,     │  │  (patterns,          │  │
│  │              │  │   edges)     │  │   conventions)       │  │
│  └──────┬───────┘  └──────┬───────┘  └──────────┬───────────┘  │
│         │                 │                      │              │
│         └─────────────────┼──────────────────────┘              │
│                           ▼                                     │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  Context Builder                                          │   │
│  │  (assembles symbols + snippets + deps for agent)         │   │
│  └──────────────────────────────────────────────────────────┘   │
│                           │                                     │
│  ┌────────────────────────▼──────────────────────────────────┐   │
│  │  Reasoning Engine                                          │   │
│  │  (pre-modification analysis, pattern discovery)            │   │
│  └────────────────────────▼──────────────────────────────────┘   │
│                           │                                     │
│  ┌────────────────────────▼──────────────────────────────────┐   │
│  │  Intelligence Diagnostics                                  │   │
│  │  (parse, index, graph, search, context metrics)            │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

### 11.2 Public Traits

| Trait | Module | Purpose |
|-------|--------|---------|
| `CodeParserTrait` | `parser` | Language-agnostic source parsing |
| `SymbolDatabaseTrait` | `index` | Persistent symbol storage |
| `CodeIndexerTrait` | `index` | File/directory indexing |
| `DependencyGraphTrait` | `graph` | Code dependency representation |
| `SemanticSearchTrait` | `search` | Symbol search and ranking |
| `ContextBuilderTrait` | `context` | Context assembly for agents |
| `ReasoningEngineTrait` | `reasoning` | Pre-modification analysis |
| `IntelligenceMemoryTrait` | `memory` | Project knowledge persistence |
| `LspFoundationTrait` | `lsp` | LSP protocol foundation |
| `IntelligenceDiagnosticsTrait` | `diagnostics` | Platform health monitoring |

### 11.3 Rules

1. **The intelligence layer is read-only.** It never writes files or executes commands.
2. **The symbol database is SQLite-based.** Schema changes require a migration ADR.
3. **Indexing is incremental.** Only changed files are re-indexed.
4. **The intelligence layer is available for P4 integration.** It is now wired into the platform architecture.
5. **LSP interfaces are stubs only.** They define the contract for future LSP server implementation.
6. **All components expose formal traits.** Consumers depend on traits for future swap-in capability.
7. **Diagnostics are recorded for all public operations.** Platform health is observable.

### 11.4 ADR Reference

The Intelligence Platform architecture is defined by **ADR-008**. Any changes to module boundaries, trait signatures, or data flow require an approved ADR.

---

## 12. Architectural Contracts

### 12.1 Module-to-Module Contracts

| From | To | Contract |
|------|----|----------|
| `tui/ui.rs` | `tools::executor` | Call `run_tool_pipeline(task, root)` → `PipelineResult` |
| `tui/ui.rs` | `agent::coordinator` | Call `coordinator.run_task(task, root, &emit)` → `String` report |
| `tui/ui.rs` | `providers` | Use `Provider::stream_response()` — NOT raw reqwest |
| `agent::coordinator` | `agent::subagent::*` | Pass `SubAgentContext`; receive `SubAgentResult` |
| `agent::coordinator` | `agent::events` | Emit `AgentEvent` via closure |
| `agent::coordinator` | `tools::executor` | No direct call — tools are executed in `tui/ui.rs` before coordinator |
| `session::SessionTracker` | `agent::events` | Receive cloned `AgentEvent` to record |
| `intelligence::*` | `tools::executor` | Future: search results feed into pipeline context |

### 12.2 Prohibited Contracts

| From | To | Reason |
|------|----|--------|
| `tui/` | `providers/` (raw reqwest) | Must use Provider trait |
| `agent/` | `tools/` (direct) | Must go through executor pipeline |
| `config/` | `agent/` | Config is loaded before agent exists |
| `intelligence/` | `tools/` | Intelligence is read-only |
| Any module | `main.rs` | main.rs is entry point only |

---

## 13. Freeze Checklist

All items below are frozen. Any change requires an approved ADR.

- [x] Module boundaries defined in Section 3
- [x] Provider trait contract defined in Section 4
- [x] Tool trait contract defined in Section 5
- [x] Event system design defined in Section 6
- [x] Memory three-tier architecture defined in Section 7
- [x] Session lifecycle defined in Section 8
- [x] Configuration sources defined in Section 9
- [x] TUI layout model defined in Section 10
- [x] Intelligence layer boundaries defined in Section 11
- [x] Intelligence layer traits defined (ADR-008)
- [x] Intelligence diagnostics module added
- [x] Module-to-module contracts defined in Section 12

---

## 14. ADR Requirements

The following types of changes **require an ADR** before implementation:

1. Adding a new top-level module under `src/`
2. Changing the signature of `Provider` or `Tool` traits
3. Adding a new `AgentEvent` variant
4. Changing the memory JSON schema
5. Changing the session JSON schema
6. Adding a new dependency to `Cargo.toml`
7. Moving code between modules
8. Changing the event flow between major subsystems
9. Modifying the TUI layout model (adding/removing panels)
10. Changing the configuration schema

---

## 15. References

- [SOP v1.0](../SOP/codebro_sop_v1.md)
- [Development Protocol](../SOP/development_protocol.md)
- [Architecture Decision Record Template](../ADR/template.md)
