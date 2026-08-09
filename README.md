# CodeBro

Your AI coding partner in the terminal.

CodeBro is a lightweight Claude Code inspired terminal coding agent built with Rust. It allows developers to chat with AI models, understand repositories, inspect code, modify files, execute commands, review changes, and maintain coding sessions.

## Features

- **AI Chat**: Interactive chat interface in the terminal
- **Streaming Responses**: Real-time token streaming from AI providers
- **Markdown Rendering**: Rich markdown display in the terminal
- **Tool Integration**: Filesystem, shell, and git tools
- **Repository Indexing**: Incremental repository indexing with .gitignore support
- **Context Building**: Intelligent context selection with token budget management
- **Patch Engine**: Unified diff-based file editing with preview and rollback
- **Tool Dispatcher**: Extensible tool registry with runtime dispatch
- **Project Scanner**: Automatic language, framework, and build system detection
- **Session Memory**: Persistent conversation history and session management
- **Advanced Memory**: Short-term, project, and global long-term memory layers
- **Memory Consolidation Engine**: Duplicate detection, similar memory merging, outdated cleanup, low-value removal
- **Planning Memory**: Reusable plan storage with similarity search and confidence scoring
- **Skill System**: Auto skill creation, discovery, ranking, usage tracking, and lifecycle management
- **Skill Lifecycle**: Draft → Testing → Trusted → Deprecated progression with confidence scoring
- **Skill Validation**: Project compatibility, language/framework matching, success rate checks
- **Skill Conflict Resolution**: Priority-based ranking by project specificity, recency, success rate, confidence
- **Permission Safety Layer**: Allow/deny/ask permission levels with dangerous pattern detection
- **Agent Operation Trace**: Operational tracing with task IDs, tool execution, and lessons learned
- **Workspace Awareness**: Project context tracking with active files, recent commands, and workspace metadata
- **Shell Session Improvements**: Command timeout, working directory tracking, environment tracking, command history
- **Reflection Engine**: Post-task reflection with lessons learned storage
- **Adaptive Planner**: Memory-aware planning with skill and plan reuse, reasoning explanation
- **Multiple Providers**: OpenAI, OpenRouter, DeepSeek, Ollama, LM Studio

## Architecture

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
    │   ├── agent.rs
    │   ├── planner.rs
    │   ├── memory.rs
    │   ├── skill.rs
    │   ├── reflection.rs
    │   └── plan_memory.rs
    ├── providers/
    │   ├── mod.rs
    │   ├── provider.rs
    │   └── openai.rs
    ├── tools/
    │   ├── mod.rs
    │   ├── filesystem.rs
    │   ├── shell.rs
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
> See the current canonical architecture in `docs/architecture/`.

### Agent Workflow

1. **User Input** → CLI/TUI captures request
2. **Memory Search** → Search short-term, project, and global memory for context
3. **Skill Search** → Find matching skills from `.codebro/skills/`
4. **Plan Memory** → Check for reusable successful plans
5. **Project Scan** → Detect language, framework, build system
6. **Repository Index** → Build `.codebro/index.json` with file metadata
7. **Context Building** → Select relevant files within token budget
8. **Prompt Assembly** → System prompt + conversation + project summary + relevant files
9. **Model Inference** → Send assembled prompt to provider
10. **Tool Dispatch** → Execute tools via dispatcher
11. **Patch Engine** → Apply changes with unified diff preview
12. **Verify** → Validate changes
13. **Reflect** → Analyze what worked, what failed, lessons learned
14. **Update Memory** → Store lessons, update skills, record plan usage
15. **Auto Skill Creation** → Extract reusable skills from successful patterns

### Repository Indexing

CodeBro automatically indexes the repository to generate `.codebro/index.json`:

```json
{
  "entries": [
    {
      "path": "src/main.rs",
      "language": "rust",
      "size": 1024,
      "last_modified": "2024-01-01T00:00:00Z",
      "hash": "abc123...",
      "ignored": false
    }
  ],
  "root": "/path/to/project",
  "generated_at": "2024-01-01T00:00:00Z"
}
```

**Features:**
- Respects `.gitignore` patterns
- Auto-ignores `target/`, `node_modules/`, `dist/`, `build/`, `.git/`
- Detects binary files
- Incremental refresh (only changed files)
- Language detection by extension

### Context Building

The agent never sends the entire repository. Instead, it:

1. Takes user request and repository index
2. Scores files by relevance (path match, content match, directory importance)
3. Filters by penalties (test files, build artifacts, vendor)
4. Fits selection within configurable token budget
5. Returns ranked relevant files

### Patch Engine

Patch-based editing replaces direct file overwrite:

1. Read file → Generate patch → Preview diff → Apply → Verify
2. Supports unified diff format
3. Multiple file patches in one operation
4. Rollback on failure

### Tool System

Tools are separated from the agent via a dispatcher:

- **Tool Registry**: Register tools by name
- **Dispatcher**: Runtime execution with error handling
- **Adding tools**: Only requires registration

```rust
let registry = ToolRegistry::new()
    .register(Arc::new(ReadFile))
    .register(Arc::new(EditFile));

let dispatcher = ToolDispatcher::new(registry);
let result = dispatcher.dispatch("read_file", "src/main.rs").await?;
```

### Session Memory

Memory is persisted to `.codebro/memory.json`:

- Conversation history with timestamps
- Recent files, commands, plans
- Multiple named sessions
- Resume previous sessions
- Clear memory

### Advanced Memory System

CodeBro uses a three-tier memory architecture:

**Short-term Memory** (in-memory, limited to 100 entries):
- Recent conversation entries
- Automatically pruned when limit exceeded
- Fast access for current session context

**Project Memory** (`.codebro/memory.json`):
- Project summary
- Recent files, commands, plans
- Tasks (pending, in-progress, completed, failed)
- Decisions with rationale
- Coding preferences (language, framework, style)

**Global Long-term Memory** (`.codebro/memory.json`):
- Successful solutions to past problems
- Lessons learned from failures
- Reflection history
- Cross-project knowledge

### Skill System

Skills are reusable workflows stored in `.codebro/skills/`:

```markdown
# Skill: add_login_page

## Description
Add a login page to a web application

## Trigger Conditions
- "add login"
- "create login page"
- "authentication page"

## Workflow
1. Inspect existing pages
2. Create login component
3. Add routes
4. Run tests

## Examples
- "Add a login page to the React app"

## Tools Used
- list_files, read_file, create_file, edit_file, run_command

## Files Changed
- src/pages/Login.tsx
- src/routes.ts

## Confidence
0.85 (used 13 times, 11 successful)
```

**Features:**
- Auto skill extraction from completed tasks
- Skill discovery by trigger matching
- Confidence scoring based on success rate
- Usage tracking and ranking

### Agent Learning Workflow

```
User Request
    ↓
Memory Search (short-term, project, global)
    ↓
Skill Search (find matching skills)
    ↓
Plan Memory Search (find reusable plans)
    ↓
Create/Reuse Plan
    ↓
Execute with Tools
    ↓
Verify Results
    ↓
Reflect (what worked, what failed)
    ↓
Update Memory (lessons, solutions)
    ↓
Auto Skill Creation (if pattern detected)
    ↓
Record Plan Usage (for future reuse)
```

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

```bash
codebro
```

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Ctrl+C` | Exit |
| `Ctrl+L` | Clear screen |
| `Ctrl+S` | Save session |
| `Up/Down` | Scroll conversation |

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
│   ├── cli/                 # CLI parsing
│   │   └── mod.rs
│   ├── tui/                 # Terminal UI
│   │   ├── mod.rs
│   │   ├── app.rs           # App state
│   │   ├── events.rs        # Event loop
│   │   └── ui.rs            # Rendering
│   ├── agent/               # Agent core
│   │   ├── mod.rs
│   │   ├── agent.rs         # Agent orchestration
│   │   ├── planner.rs       # Task planning
│   │   └── memory.rs        # Session memory
│   ├── providers/           # LLM providers
│   │   ├── mod.rs
│   │   ├── provider.rs      # Provider trait
│   │   └── openai.rs        # OpenAI implementation
│   ├── tools/               # Tool system
│   │   ├── mod.rs           # Tool trait
│   │   ├── filesystem.rs    # File operations
│   │   ├── shell.rs         # Shell commands
│   │   ├── git.rs           # Git operations
│   │   └── patch.rs         # Patch engine
│   ├── indexer/             # Repository indexer
│   │   ├── mod.rs
│   │   └── scanner.rs       # File indexing
│   ├── scanner/             # Project scanner
│   │   ├── mod.rs
│   │   └── project.rs       # Project detection
│   ├── dispatcher/          # Tool dispatcher
│   │   ├── mod.rs
│   │   └── registry.rs      # Tool registry
│   └── config/              # Configuration
│       └── mod.rs
└── tests.rs                 # Integration tests
```

> The legacy `src/context/` (context builder) and `src/prompt/` (prompt
> assembly) modules were removed in ADR-012. Canonical replacements:
> `src/assembly/` + `src/engineering_context/` and `src/prompt_builder/`
> (`compile_context(&EngineeringContext)`).

## License

MIT

---

## CodeBro v0.4 Reliability Layer

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

## TUI Agent Command Center (v0.6.5)

CodeBro v0.6.5 upgrades the TUI from a basic chat interface into a real-time autonomous agent dashboard.

### Dashboard Layout

```
+--------------------------------+
| CodeBro Agent Dashboard        |
+--------------------------------+
| Chat                           |
|                                |
+--------------------------------+
| Agents                         |
|                                |
| Research   ✓ completed         |
| Planning   ✓ completed         |
| Coding     ⟳ executing         |
| Testing    waiting             |
| Review     waiting             |
+--------------------------------+
| Activity Log                   |
|                                |
| searching symbols...           |
| generating patch...            |
| running tests...               |
+--------------------------------+
| Input                          |
+--------------------------------+
```

### Live Agent Monitoring

Every agent exposes its current state in real-time:
- **Status**: idle, thinking, searching, analysing, planning, executing, testing, reviewing, completed, failed
- **Current Task**: what the agent is working on
- **Progress**: percentage with animated progress bar
- **Latest Action**: most recent action taken

### Agent Event Bus

Events are published and consumed by the TUI, trace system, and logger:

- `AgentStarted` - agent begins a task
- `AgentProgress` - agent progress update
- `ToolStarted` / `ToolCompleted` - tool execution
- `TaskUpdated` - task graph changes
- `MemoryUpdated` - memory consolidation
- `SkillUpdated` - skill confidence changes
- `AgentCompleted` / `AgentFailed` - agent lifecycle

### Task Visualization

Toggle the task graph view with `Ctrl+G`:

```
Task: Refactor authentication
Graph:
✓ Research
|
✓ Planning
|
⟳ Coding
|
○ Testing
|
○ Review
```

### Live Animations

- **Thinking spinner**: `⠋ ⠙ ⠹ ⠸ ⠼ ⠴`
- **Progress indicators**: `██████░░░░`
- **Activity animations**: Researching..., Searching..., Building...

Animations are non-blocking and use async tick events.

### Tool Execution View

Displays active tools with name, arguments summary, and result. Secrets are redacted.

### Memory and Skill Notifications

- **Memory Updated**: displays new learned information
- **Skill Updated**: shows confidence changes (e.g., `0.82 -> 0.89`)

### Streaming Response UI

AI responses appear progressively as they stream, rather than waiting for the full response.

### Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+A` | Toggle agent panel |
| `Ctrl+G` | Toggle task graph |
| `Ctrl+M` | Show memory changes |
| `Ctrl+S` | Save session |
| `Ctrl+T` | Show trace |
| `Ctrl+L` | Clear logs |
| `Ctrl+C` | Cancel current task |
| `Ctrl+P` | Open command palette |
| `Ctrl+V` | Toggle metrics panel |
| `Ctrl+O` | Toggle coordination view |
| `Ctrl+Q` | Quit |

## Agent Coordination Layer (v0.7)

CodeBro v0.7 transforms the multi-agent system into a coordinated autonomous agent team with communication, dynamic planning, and shared knowledge.

### Agent Message Bus

Agents communicate via an async message bus supporting:

- **ResearchResult** - findings from codebase analysis
- **PlanningUpdate** - plan modifications based on discoveries
- **CodeChangeProposal** - proposed modifications with risk assessment
- **ReviewFeedback** - security and quality concerns
- **TestResult** - test execution outcomes
- **RecoveryRequest** - failure escalation
- **DecisionRequest** - conflict resolution

Example:
```
Research Agent: "Found authentication logic in middleware.rs"
    ↓
Planning Agent: "Updating plan - modify middleware instead of auth.rs"
```

### Shared Agent Workspace

`.codebro/workspace/` stores shared artifacts:
- `research.json` - codebase findings
- `plan.json` - implementation plans
- `decisions.json` - resolved conflicts
- `changes.json` - pending code modifications
- `test_results.json` - test outcomes
- `review.json` - code review findings

### Dynamic Task Replanning

The Task Graph now supports real-time updates:
- **Add task** - new steps discovered during execution
- **Remove task** - tasks no longer needed
- **Reorder task** - priority changes based on context
- **Change dependencies** - dynamic dependency resolution

Example:
```
Original: Modify auth.rs
Discovery: Auth logic moved to middleware.rs
Update: Modify middleware.rs instead
```

### Agent Coordinator

Central coordination responsible for:
- Spawning and managing agents
- Assigning tasks based on capabilities
- Collecting results
- Resolving conflicts via decision engine

Flow:
```
Main Agent → Coordinator → Subagents → Coordinator → Main Agent
```

### Agent Decision System

When conflicts occur:
```
Coding Agent: "Use approach A"
Review Agent: "Security risk - use approach B"
Decision Engine:
  - Evaluate previous experiences
  - Check skill confidence
  - Consider project patterns
  → Choose approach B (security-first)
```

### Resource Management

Intelligent resource allocation:
- **Small task**: 1-2 agents
- **Medium task**: 2-3 agents
- **Large task**: Full team (5-6 agents)

Tracks:
- Token budget
- Execution time limits
- Priority queue
- Agent utilization

### Agent Performance Learning

Each agent learns from experience:
```
Coding Agent:
  Rust refactor:    success 94% (124 tasks)
  Python migration: success 72% (45 tasks)
```

Routing improves based on domain expertise.

### Improved Recovery Coordination

Team-based recovery:
```
Testing Agent: cargo test failed
  ↓
Recovery Engine: analyze failure
  ↓
Coding Agent: fix the issue
  ↓
Review Agent: check for regressions
  ↓
Testing Agent: retry tests
```

### Coordination View (Ctrl+O)

```
Agent Communication
Research:   Found auth flow in middleware.rs
Planning:   Updated task graph
Coding:     Applying middleware patch
Review:     Security concern detected

Agent Performance
Coding Agent    Success: 91%  Tasks: 124
Research Agent  Success: 95%  Tasks: 89
```

### Example Coordinated Workflow

```
User: "Refactor authentication to use middleware"

1. Research Agent searches codebase
   → Discovers auth logic in middleware.rs
   → Sends ResearchResult to Coordinator

2. Planning Agent updates plan
   → Changes target from auth.rs to middleware.rs
   → Sends PlanningUpdate

3. Coding Agent proposes changes
   → Sends CodeChangeProposal with risk assessment

4. Review Agent identifies security concern
   → Sends ReviewFeedback (High severity)

5. Decision Engine resolves conflict
   → Chooses security-first approach

6. All agents coordinate via message bus
   → Shared workspace updated
   → Task graph dynamically adjusted
```

## Real Usage Hardening (v0.6.6)

CodeBro v0.6.6 prepares the project for real daily developer usage with improved observability, reliability, trust, debugging, cost awareness, and code change safety.

### Session Replay System

Persistent task sessions stored in `.codebro/sessions/`:

```json
{
  "id": "...",
  "created_at": "...",
  "task": "Add caching",
  "agents": ["research", "coding", "testing"],
  "timeline": [...],
  "tools_used": [...],
  "files_changed": [...],
  "result": "success",
  "lessons": [...]
}
```

Each session tracks agent events, tool executions, decisions, errors, and final outcomes. Replay any session timeline to understand exactly what happened.

Commands:
- `/sessions` - List all sessions
- `/replay <id>` - Replay a session timeline

### Execution Metrics

Track detailed task metrics:
- **Total duration** - overall task time
- **Agent duration** - time per agent
- **Tool duration** - time per tool
- **Token usage** - input + output tokens
- **Context size** - context window usage
- **Files modified** - files changed during task
- **Retry count** - number of retries

Displayed in the TUI via the metrics panel (Ctrl+V).

### Provider Cost Tracking

Track provider usage across sessions:
- **Input tokens** per request
- **Output tokens** per request
- **Estimated cost** per model
- **Model used**

Supported models include: GPT-4o, GPT-4, GPT-3.5, Claude (Opus/Sonnet/Haiku), DeepSeek, Gemini.

Usage history stored in `.codebro/usage.json`.

### Improved Error Recovery

Automatic failure handling with:
1. **Failure detection** - classify errors (compile, test, permission, timeout, provider)
2. **Failure analysis** - determine root cause and retryability
3. **Retry strategy** - automatic retry for transient failures (timeout, provider errors)
4. **Escalation** - escalate repeated failures to appropriate agents

Example flow:
```
Testing Agent: cargo test failed
Recovery:
1. Analyse error (test failure)
2. Ask Coding Agent to fix
3. Retry test
4. Escalate if repeated failure
```

Recovery attempts stored in `.codebro/recovery.json`.

### Terminal Diff Review

Before applying destructive changes, review the diff:

```
File: src/auth/service.rs
Diff:
- old line
+ new line

Actions:
Y  Accept
N  Reject
E  Edit
```

This provides user approval before destructive changes are applied.

### Command Palette

Press `Ctrl+P` to open the command palette:

```
/help       Show help
/sessions   List sessions
/replay     Replay a session
/agents     Show agent status
/tasks      Show task graph
/memory     Show memory changes
/skills     Show skill changes
/metrics    Show task metrics
```

### Dashboard Metrics Panel

Press `Ctrl+V` to toggle the metrics panel:

```
Task Metrics
Agents:    5
Progress:  72%
Tokens:    18k
Cost:      $0.12
Time:      04:32
```

### Example Daily Workflow

```
Developer: "Add caching to the API layer"

1. Research Agent searches codebase → finds API handlers, DB layer
2. Planning Agent creates implementation plan
3. Coding Agent applies changes (diff review: Y to accept)
4. Testing Agent runs cargo test → fails on edge case
5. Recovery: analyze failure → Coding Agent fixes → retry → passes
6. Review Agent validates the implementation
7. Session saved to .codebro/sessions/
8. Metrics recorded: 4 agents, 12 tools, 32k tokens, $0.12
9. Skill 'rust-api' confidence improves: 0.82 -> 0.89
```
