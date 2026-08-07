# Changelog

All notable changes to CodeBro will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [1.0.0] - 2026-08-06

### Added
- **P6 Foundation Platform**
  - Preference Engine: Persistent, schema-versioned preference storage with atomic writes and rollback
  - Intent Engine: Deterministic intent classification with regex-based rules
  - Recommendation Engine: Rule-based recommendations from intent plans
  - Workflow Engine: Deterministic workflow planning with dependency analysis
  - Adaptive Validation: Read-only pipeline validation with policy-driven rules
- **P7 Release Candidate**
  - Integration Pipeline: End-to-end orchestration of all P6 engines
  - PipelineResult: Immutable, serializable pipeline output
  - ApprovalSummary: Human-readable approval view for TUI
  - Concurrency tests: Thread-safety and determinism verification
- **P8 Stable**
  - Production packaging and release artifacts
  - Comprehensive documentation (19 reports)
  - CHANGELOG and Release Notes

### Features
- Multi-agent architecture with Research, Planning, Coding, Testing, Review agents
- Tree-sitter code indexing for Rust, Python, JavaScript, TypeScript, Go
- Semantic code search with relevance ranking
- Dependency graph analysis
- Memory consolidation engine (dedup, merge, cleanup)
- Skill lifecycle system (Draft → Testing → Trusted → Deprecated)
- Permission safety layer with dangerous pattern detection
- Agent operation tracing
- Workspace awareness
- Session replay system
- Execution metrics and cost tracking
- Terminal diff review with accept/reject/edit
- Command palette (Ctrl+P)
- Dashboard metrics panel (Ctrl+V)
- Agent coordination view (Ctrl+O)
- Streaming responses in TUI
- Live agent status monitoring
- Task graph visualization (Ctrl+G)

### Configuration
- Zero-configuration first run
- Environment variable support (`CODEBRO_API_KEY`, `CODEBRO_BASE_URL`, `CODEBRO_MODEL`)
- TOML config file (`~/.codebro/config.toml`)
- Multi-provider support (OpenAI, OpenRouter, DeepSeek, Ollama, LM Studio)

### CLI Commands
- `codebro` — Start TUI chat
- `codebro chat` — Start TUI chat (explicit)
- `codebro list-models` — List available models
- `codebro onboard` — Run onboarding wizard

### Keyboard Shortcuts
- `Ctrl+A` — Toggle agent panel
- `Ctrl+G` — Toggle task graph
- `Ctrl+M` — Show memory changes
- `Ctrl+S` — Save session
- `Ctrl+T` — Show trace
- `Ctrl+L` — Clear logs
- `Ctrl+C` — Cancel current task
- `Ctrl+P` — Open command palette
- `Ctrl+V` — Toggle metrics panel
- `Ctrl+O` — Toggle coordination view
- `Ctrl+Q` — Quit

### Changed
- None (first stable release)

### Deprecated
- None

### Removed
- None

### Fixed
- Recommendation latency threshold adjusted for consistent test timing

### Security
- Permission safety layer prevents dangerous operations
- API keys never logged or persisted in config
- Atomic preference writes prevent corruption
- Backup/rollback on corruption detection

### Performance
- Single pipeline latency: ~0.95ms
- Multi-threaded throughput: ~11.7K ops/ms
- Peak memory: ~2.3 MB (single), ~18.5 MB (100 threads)
- Determinism verified: 0.00% deviation

### Dependencies
- ratatui 0.26 — Terminal UI
- crossterm 0.27 — Terminal interaction
- tokio 1 — Async runtime
- reqwest 0.12 — HTTP client
- tree-sitter 0.20 — Code parsing
- rusqlite 0.31 — SQLite database
- clap 4 — CLI parsing

---

## [0.7.0] - 2026-07-28

### Added
- Agent Coordination Layer (v0.7)
- Agent Message Bus
- Shared Agent Workspace
- Dynamic Task Replanning
- Agent Decision System
- Resource Management
- Agent Performance Learning

---

## [0.6.5] - 2026-07-20

### Added
- TUI Agent Command Center (v0.6.5)
- Dashboard layout with agent panel, activity log
- Live agent monitoring
- Agent event bus
- Task visualization
- Live animations
- Tool execution view
- Memory and skill notifications
- Streaming response UI

---

## [0.6.0] - 2026-07-10

### Added
- Multi-agent architecture (v0.6)
- Subagent framework (Research, Planning, Coding, Testing, Review)
- Task Router with complexity analysis
- Task Graph Engine with DAG representation
- Experience Replay system
- Smart Tool Router

---

## [0.5.0] - 2026-06-28

### Added
- Code Intelligence Architecture (v0.5)
- Tree-sitter integration for 5 languages
- Symbol index with SQLite
- Semantic code search
- Dependency graph
- Intelligent context builder
- LSP foundation

---

## [0.4.0] - 2026-06-15

### Added
- Reliability Layer (v0.4)
- Memory Consolidation Engine
- Skill Lifecycle system
- Permission Safety Layer
- Agent Operation Trace
- Shell Session improvements
- Workspace Awareness

---

## [0.3.0] - 2026-06-01

### Added
- Tool system with dispatcher
- Patch engine for file editing
- Repository indexing
- Context building
- Session memory

---

## [0.2.0] - 2026-05-15

### Added
- TUI with chat interface
- Provider abstraction
- Streaming responses
- Markdown rendering

---

## [0.1.0] - 2026-05-01

### Added
- Initial project structure
- Basic CLI
- Config system

---

[1.0.0]: https://github.com/afnanrudy/codebro/releases/tag/v1.0.0
[0.7.0]: https://github.com/afnanrudy/codebro/releases/tag/v0.7.0
[0.6.5]: https://github.com/afnanrudy/codebro/releases/tag/v0.6.5
[0.6.0]: https://github.com/afnanrudy/codebro/releases/tag/v0.6.0
[0.5.0]: https://github.com/afnanrudy/codebro/releases/tag/v0.5.0
[0.4.0]: https://github.com/afnanrudy/codebro/releases/tag/v0.4.0
[0.3.0]: https://github.com/afnanrudy/codebro/releases/tag/v0.3.0
[0.2.0]: https://github.com/afnanrudy/codebro/releases/tag/v0.2.0
[0.1.0]: https://github.com/afnanrudy/codebro/releases/tag/v0.1.0
