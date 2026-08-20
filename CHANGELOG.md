# Changelog

All notable changes to CodeBro will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.7.0-mcp-rc2] - 2026-08-17

> **Status:** Release candidate for real-world agent dogfooding and stabilization.

### Added
- **M1: Engineering Memory Trust** — Trust scores on memory entries computed from confidence, importance, and freshness. `engineering_memory` responses include per-entry `trust` metadata. `memory_stats` reports average trust. Backward-compatible: existing entries without freshness data use `unknown` freshness status.
- **M2: Change Invalidation Advisory** — `apply_change` now returns `needs_reindex` advisory when source files in the fact store are affected. Impacted fact IDs are correlation metadata (not independently verified causal evidence). `impact_analyze` provides structural relationship edges (callers, importers, references) for context.
- **M3: Lightweight Evidence Chaining** — `impact_analyze` returns directed relationship edges with provenance metadata. Evidence chain: `engineering_facts` → `impact_analyze` → `apply_change` → invalidation advisory → `reindex` → fresh FactStore → `sandbox_test`/`sandbox_build` → `VerificationResult`.
- **M4: MCP Reindex Trigger** — `reindex` tool performs full fact-store regeneration via the existing `codebro init` pipeline. Returns `status`, `fact_counts`, `generation_repo_state`, `validation`, and `duration_ms`. This is a full rebuild, not incremental indexing.
- **M5: Repository Health MCP Tool** — `repository_health` exposes the existing `codebro doctor` capability as MCP tool #14. Read-only; returns structured JSON with `exit_code`, `status`, `checks`, and `summary`. Delegates directly to the existing doctor runtime without adding new checks, auto-repair, or orchestration. All 6 existing doctor checks (workspace_root, .codebro, project_identity, facts, engineering_memory, git) preserved with exact semantics and exit-code values (0=healthy, 1=warn, 2=error).

### Summary
- **15 MCP tools** (exact count: RC2 had 14 + M5 addition of `repository_health` + this release's `consult`)
- **3186 tests passing**, 0 failed, 11 ignored
- **MCP-first architecture** — host agent owns planning/orchestration; CodeBro owns engineering truth/infrastructure
- **Stabilization / dogfooding phase** — M6 has NOT started

### Security
- **Sprint 28 hardening** — credential lifecycle and execution integrity:
  - `//apikey` no longer accepts an inline key. Run `//apikey [provider]` and
    enter the secret in a masked prompt (`Enter` to store, `Esc` to cancel);
    inline keys are rejected and never enter input history or context.
  - `CredentialStore` (`~/.codebro/credentials.json`) persists atomically with
    mode `0600`, refuses symlinked paths, fsyncs before rename, and surfaces
    security-critical failures instead of ignoring them. `Debug` output
    exposes provider presence only, never values.
  - Secrets are redacted before reaching shell history, session files,
    conversation/context, input history, exports, clipboard text, and
    activity logs via the single tool redaction authority
    (`redact_secrets_public`), extended with password/secret/token, GitHub/
    GitLab/Slack token, and URL-credential patterns.
  - `read_file` tool output is redacted so a workspace credential file cannot
    leak into model context.

### Fixed
- **Sprint 28 hardening** — blocking shell execution (`execute_child`)
  drained stdout/stderr while the child runs, eliminating pipe-buffer
  deadlocks on large output; output stays bounded; timeouts terminate the
  whole process group; PTY/stream thread-creation failures are surfaced as
  errors instead of silently dropped.
- **`codebro init` memory scaling** — eliminated O(n²) combinatorial explosion
  in heuristic reference/relationship generation (`src/impact/relationships.rs`).
  Heuristic edges are now bounded to symbol pairs whose modules already share
  a verified AST-derived relationship (call or import), preventing common-name
  collision blowups (e.g. `new`, `tests`, `fmt`). Added early drop of
  intermediate collected vectors (`collected_modules`, `collected_symbols`,
  `all_calls`, `all_imports`, `files`) before serialization. Replaced
  `serde_json::to_string_pretty()` + `write()` with streaming
  `serde_json::to_writer_pretty()` to avoid a full second copy of the model
  in RAM during JSON generation. Before/after on the CodeBro repo (348 source
  files): peak RSS 1,294 MB → 753 MB (−42%); facts.json 216 MB → 13.6 MB
  (−94%); references 292,474 → 0; relationships 88,593 → 6,014. Atlas
  (36 Rust files): peak RSS 29 MB → 22 MB; facts.json 216 KB → 909 KB.
  All 3140 tests passing; determinism verified.

## [Unreleased]

### Changed
- **Consultant cleanup** — Removed dead `ChatGpt`, `Claude`, and `DeepSeek` variants from `ConsultantProvider` enum. Removed stale doc comments referencing removed browser/extension providers. Fixed unreachable-pattern clippy error in `ConsultantRouter::resolve`. Prompt builder and provider docs no longer reference ChatGPT/Claude/DeepSeek.

### Added
- **Conductor consultant provider** — `ConductorProvider` (`src/consultant/providers/conductor.rs`) is now the primary and only supported consultant runtime. CodeBro calls Conductor's OpenAI-compatible `POST /v1/chat/completions` endpoint with `Authorization: Bearer <CONDUCTOR_API_KEY>`. Configuration via `CONDUCTOR_API_KEY`, `CONDUCTOR_BASE_URL` (default `http://127.0.0.1:8080`), and `CONDUCTOR_MODEL` env vars (or the secure credential store). CLI: `codebro consult --provider conductor --mode <mode> "question"`. MCP tool: `consult` (tool #15). Mode mapping: `architecture→agentic`, `debugging→coding`, `code_review→coding`, `planning→planning`, `research→reasoning`, `second_opinion→reasoning`.
- **MCP `consult` tool (tool #15)** — Ask an AI consultant (Conductor gateway) for opinions. Supports `provider` (`auto` | `conductor`), `mode`, `question`, optional `context`, `files`, `include_git_diff`, `include_project_context`, `max_answer_length`. Project context and git diff are injected automatically when requested.
- **CLI `codebro consult`** — Same capability as the MCP tool from the terminal.
- **CLI `codebro auth status`** — Shows authentication status for registered consultant providers.
- **Sprint 29 — Consultant architecture** — `src/consultant/` module with `ConsultantProvider` trait, `ConsultantRouter`, shared `build_prompt`/`truncate_answer`, type-safe mode/provider enums.

### Removed
- **Browser-based consultant providers removed** — Firefox WebExtension bridge, extension bridge server, and bridge daemon are removed. The ChatGPT extension provider, legacy Playwright-based ChatGPT provider, and browser-profile-based Claude/DeepSeek stub providers are removed. `codebro bridge start/stop/status` CLI commands are removed. `codebro auth login/logout` browser flows are removed. Supported consultant runtime is now API-first via Conductor only.
- **Sprint 25 — Architecture Consolidation (ADR-012)**
  - Removed legacy `src/context/` (v0.3 context builder) — superseded by `engineering_context` + `assembly`.
  - Removed legacy `src/prompt/` (v0.3 prompt assembly) — zero consumers; superseded by `prompt_builder`.
  - Removed `intelligence/memory/` (`IntelligenceMemory`) — dead duplicate of `project_identity` / `engineering_facts`.
  - Removed `reliability/health.rs` and `reliability/circuit_breaker.rs` — duplicates of the canonical `provider_runtime` health/circuit-breaker implementation. `reliability/` now contains only provider-agnostic generic infra.
  - Removed the legacy `PromptCompiler::compile(13 params)` and `PromptBuilder::compile()/compile_with_default_template()` APIs. `compile_context(&EngineeringContext)` is the only public compile entry point.
  - Removed `src/indexer/` (`RepositoryIndex`) — dead once its only consumers (legacy `src/context/`) were removed.
  - Removed orphaned uncompiled files: `src/tests/concurrency.rs`, `src/tests/p3_validation.rs`, `src/tests/validation.rs`, `src/memory_runtime/tests.rs`.
  - Removed ~90 tests that exercised only removed abstractions; migrated remaining tests to canonical owners.

### Added
- **Sprint 25 — Architecture Consolidation**
  - `docs/ADR/ADR-012-architecture-consolidation.md` — canonical ownership decisions for Context, Intelligence/Memory, Prompt Compiler, Provider Reliability, Task/Workflow.

### Changed
- **Sprint 25 — Architecture Consolidation**
  - `src/runtime/context.rs` no longer depends on `reliability::HealthMonitor`.
  - Documentation updated to reflect the canonical architecture (README, architecture manifest/snapshot, ADR-008/010, contracts, Reliability Architecture Report).

### Added
- **Sprint 23.0 Workspace Metadata Correction**
  - `ProjectIdentityRuntime::create` and `create_minimal` now persist the runtime's `workspace_root` into both the canonical `project_identity.json` and the `workspace.json` projection.
  - Caller-provided builder workspace roots are preserved only when they exactly match the runtime root; otherwise the runtime root wins.
  - `save_all` documentation updated to reflect sequential (non-atomic) projection writes.
  - 3 new tests; full suite 2387 passed / 0 failed
- **P10.5.1 Fact Store Foundation**
  - Canonical immutable repository for Engineering Facts built on the P10.5.0 FactsModel
  - Owns: FactStore, FactCollection, FactIndex, FactLookup, FactQuery, FactSnapshot, FactStatistics, FactDiagnostics, FactValidation
  - Deterministic, read-only primary indexes for every entity id plus reverse workspace/package/module/symbol scope indexes (pure field projections, no graph traversal)
  - Byte-identical snapshots (canonical JSON + FNV-1a 64 digest, no timestamps/randomness)
  - Store validation: duplicate facts, broken indexes, missing ids, orphan records, schema consistency
  - O(log n) allocation-free lookups and enumeration; lifecycle builder; Send + Sync, 8-thread concurrency test
  - 39 new tests; full suite 2111 passed / 0 failed
- **P10.5.0 Engineering Facts Model**
  - Immutable, language-neutral engineering fact model consumed by the Engineering Runtime
  - Facts are the only public contract between language intelligence providers and the runtime (no source, AST or parser dependency)
  - Entities: Symbol, Module, Package, Workspace, Dependency, Relationship, Reference, Test, Build Target, Diagnostic, Architecture Rule
  - Opaque IDs (`FactId`) with no UUID generation, timestamps or randomness
  - Deterministic validation: duplicate IDs, invalid references, self-references, orphan symbols, broken dependency links, unresolved visibility
  - `FactsBuilder → FactsModel` freeze pattern; id-sorted storage with O(log n) allocation-free lookups
  - Send + Sync, serde (JSON/TOML) round-trips, full determinism
  - 27 new tests; full suite 2063 passed / 0 failed

## [1.0.0] - 2026-08-06

> **Note:** This section documents the TUI-era release (pre-MCP). The current release is v0.7.0-mcp-rc2 on the MCP-first `main` branch.


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

[1.0.0]: https://github.com/EffNine/CodeBro/releases/tag/v1.0.0
[0.7.0-mcp-rc2]: https://github.com/EffNine/CodeBro/releases/tag/v0.7.0-mcp-rc2
[0.7.0-mcp-rc1]: https://github.com/EffNine/CodeBro/releases/tag/v0.7.0-mcp-rc1
[0.7.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.7.0
[0.6.5]: https://github.com/EffNine/CodeBro/releases/tag/v0.6.5
[0.6.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.6.0
[0.5.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.5.0
[0.4.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.4.0
[0.3.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.3.0
[0.2.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.2.0
[0.1.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.1.0
