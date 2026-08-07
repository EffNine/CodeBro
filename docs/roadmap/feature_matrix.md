# CodeBro Feature Matrix

**Document:** `docs/roadmap/feature_matrix.md`
**Version:** 1.0.0
**Part of:** CodeBro SOP v1.0

---

## Overview

This matrix tracks all features across the CodeBro development roadmap. Each feature is categorized, assigned a priority, and mapped to the phase where it will be implemented.

---

## Feature Categories

| Category | Description |
|----------|-------------|
| **Core Runtime** | Agent loop, LLM communication, session management |
| **Tools** | File, shell, git, and patch operations |
| **Reliability** | Error recovery, permissions, crash resistance |
| **Intelligence** | Code understanding, search, context selection |
| **UX** | TUI, keyboard navigation, discoverability |
| **Agent System** | Multi-agent coordination, parallel execution |
| **Performance** | Caching, optimization, resource management |
| **Security** | Input validation, output sanitization, sandboxing |

---

## Feature Matrix

### Core Runtime

| Feature ID | Feature | Priority | Phase | Status | Dependencies | Description |
|------------|---------|----------|-------|--------|--------------|-------------|
| FEAT-001 | Iterative agent loop | P0 | P1 | Planned | None | Replace single LLM call with ReAct loop (think→act→observe) |
| FEAT-002 | LLM streaming | P0 | P1 | Planned | None | Progressive token display in UI with cancellation support |
| FEAT-003 | Session auto-resume | P0 | P1 | Planned | None | Auto-detect and offer to resume last session on startup |
| FEAT-004 | Multi-turn context | P1 | P1 | Planned | FEAT-001 | Maintain conversation history across tool calls in a single task |
| FEAT-005 | Provider abstraction | P1 | P1 | Planned | None | Wire `Provider` trait to production streaming path |
| FEAT-006 | Task cancellation | P1 | P1 | Planned | FEAT-002 | Clean cancellation of in-flight LLM requests and tool execution |

### Tools

| Feature ID | Feature | Priority | Phase | Status | Dependencies | Description |
|------------|---------|----------|-------|--------|--------------|-------------|
| FEAT-010 | Patch-based file edits | P0 | P2 | Planned | None | Wire `ChangePlan`/`PatchEngine` into main pipeline with approval gate |
| FEAT-011 | Parallel tool execution | P1 | P3 | Planned | FEAT-010 | Execute independent tools concurrently |
| FEAT-012 | Streaming command output | P1 | P3 | Planned | None | Stream `run_command` output to UI in real-time |
| FEAT-013 | Git commit + branch | P2 | P3 | Planned | FEAT-010 | Auto-create branch and commit changes |
| FEAT-014 | Per-tool timeout | P1 | P3 | Planned | None | Configurable timeout per tool type |
| FEAT-015 | Tool output capping | P0 | P3 | Planned | None | Cap all tool output before it enters context or UI |

### Reliability

| Feature ID | Feature | Priority | Phase | Status | Dependencies | Description |
|------------|---------|----------|-------|--------|--------------|-------------|
| FEAT-020 | Permission system integration | P0 | P2 | Planned | None | Wire `PermissionManager` into main pipeline |
| FEAT-021 | Error recovery UI | P1 | P2 | Planned | None | Show retry/switch-model options on provider failure |
| FEAT-022 | Session crash recovery | P0 | P2 | Planned | FEAT-003 | Resume session state after unexpected termination |
| FEAT-023 | Retry with backoff | P1 | P2 | Planned | FEAT-021 | Automatic retry for transient failures (timeout, rate limit) |
| FEAT-024 | Graceful degradation | P2 | P2 | Planned | FEAT-021 | Fall back to simpler execution when advanced features fail |

### Intelligence

| Feature ID | Feature | Priority | Phase | Status | Dependencies | Description |
|------------|---------|----------|-------|--------|--------------|-------------|
| FEAT-030 | Semantic search in pipeline | P0 | P4 | Planned | None | Replace `grep_files()` with `SemanticSearch` |
| FEAT-031 | Symbol-aware context | P1 | P4 | Planned | FEAT-030 | Use `CodeIndexer` to select relevant symbols for context |
| FEAT-032 | Dependency-informed planning | P1 | P4 | Planned | FEAT-031 | Use `DependencyGraph` to understand change impact |
| FEAT-033 | Incremental index updates | P2 | P4 | Planned | FEAT-030 | Re-index only changed files instead of full rebuild |
| FEAT-034 | Cache-aware file search | P2 | P4 | Planned | FEAT-030 | Cache search results with invalidation on file change |

### UX

| Feature ID | Feature | Priority | Phase | Status | Dependencies | Description |
|------------|---------|----------|-------|--------|--------------|-------------|
| FEAT-040 | Inline diff display | P0 | P5 | Planned | FEAT-010 | Show diff in conversation when files are modified |
| FEAT-041 | Session browser panel | P1 | P5 | Planned | FEAT-003 | Visual session list with search and one-click replay |
| FEAT-042 | Context-aware commands | P1 | P5 | Planned | None | Filter slash commands based on current state |
| FEAT-043 | First-run wizard | P1 | P5 | Planned | None | Guide new users through config and feature tour |
| FEAT-044 | Token/cost indicator | P1 | P5 | Planned | None | Show token estimate and cost in title bar |
| FEAT-045 | Panel auto-collapse | P2 | P5 | Planned | None | Hide panels when no activity to reduce visual noise |
| FEAT-046 | Multi-line input hint | P2 | P5 | Planned | None | Show "Shift+Enter for newline" when input has multiple lines |
| FEAT-047 | Error recovery buttons | P1 | P5 | Planned | FEAT-021 | Clickable retry/switch-model buttons in error banner |

### Agent System

| Feature ID | Feature | Priority | Phase | Status | Dependencies | Description |
|------------|---------|----------|-------|--------|--------------|-------------|
| FEAT-050 | Real subagent tool execution | P0 | P6 | Planned | FEAT-011 | Subagents execute real tools, not just generate text |
| FEAT-051 | Parallel agent execution | P0 | P6 | Planned | FEAT-050 | Independent agents run concurrently |
| FEAT-052 | Dynamic task replanning | P1 | P6 | Planned | FEAT-051 | Re-plan task graph when new information emerges |
| FEAT-053 | Agent message bus in UI | P1 | P6 | Planned | FEAT-050 | Show agent-to-agent communication in coordination panel |
| FEAT-054 | Agent specialization | P2 | P6 | Planned | FEAT-050 | Agents learn domain expertise over time |
| FEAT-055 | Background task queue | P3 | P6 | Planned | FEAT-051 | Submit tasks and continue chatting |

### Performance

| Feature ID | Feature | Priority | Phase | Status | Dependencies | Description |
|------------|---------|----------|-------|--------|--------------|-------------|
| FEAT-060 | Project scan caching | P1 | P1 | Planned | None | Cache `ProjectInfo` and file listing with mtime invalidation |
| FEAT-061 | Intelligent context builder | P1 | P4 | Planned | FEAT-031 | Use intelligence layer for context selection instead of token budget |
| FEAT-062 | Batch session writes | P2 | P2 | Planned | None | Debounce session file writes to reduce I/O |
| FEAT-063 | Background index building | P2 | P4 | Planned | FEAT-033 | Build/update intelligence index in background |
| FEAT-064 | Memory usage optimization | P2 | P6 | Planned | None | Profile and reduce peak memory usage |

### Security

| Feature ID | Feature | Priority | Phase | Status | Dependencies | Description |
|------------|---------|----------|-------|--------|--------------|-------------|
| FEAT-070 | Path sandboxing | P0 | P2 | Planned | FEAT-010 | Validate all file paths against workspace root |
| FEAT-071 | Command injection prevention | P0 | P3 | Planned | FEAT-012 | Sanitize shell command arguments |
| FEAT-072 | Secret redaction expansion | P1 | P3 | Planned | FEAT-012 | Add more secret patterns to redaction regex |
| FEAT-073 | Config encryption | P3 | P7 | Planned | None | Encrypt API keys in config file |

---

## Priority Definitions

| Priority | Definition | Implementation Rule |
|----------|------------|---------------------|
| **P0** | Blocking — feature must exist for the phase to be considered complete | Must implement in the assigned phase |
| **P1** | Important — feature significantly improves the phase outcome | Should implement; can defer with justification |
| **P2** | Nice to have — feature improves quality but is not essential | Can defer to a later phase |
| **P3** | Future — feature is desirable but not needed for release | Schedule after stable release |

---

## Status Definitions

| Status | Definition |
|--------|------------|
| **Planned** | Feature is in the matrix but not yet started |
| **In Progress** | Feature is being implemented in the current phase |
| **Validated** | Feature is implemented and passed validation |
| **Deferred** | Feature is postponed to a later phase (with reason) |
| **Cancelled** | Feature is removed from the roadmap (with reason) |
| **Superseded** | Feature is replaced by a different approach |

---

## Phase-to-Feature Mapping

### P1 — Core Runtime
- FEAT-001, FEAT-002, FEAT-003, FEAT-004, FEAT-005, FEAT-006
- FEAT-060 (performance: project scan caching)

### P2 — Reliability Layer
- FEAT-010, FEAT-020, FEAT-021, FEAT-022, FEAT-023, FEAT-024
- FEAT-070, FEAT-071
- FEAT-062 (performance: batch writes)

### P3 — Tool Engine
- FEAT-011, FEAT-012, FEAT-013, FEAT-014, FEAT-015
- FEAT-072 (security: expanded redaction)

### P4 — Intelligence Layer
- FEAT-030, FEAT-031, FEAT-032, FEAT-033, FEAT-034
- FEAT-061 (performance: intelligent context)

### P5 — UX Foundation
- FEAT-040, FEAT-041, FEAT-042, FEAT-043, FEAT-044, FEAT-045, FEAT-046, FEAT-047

### P6 — Advanced Agent System
- FEAT-050, FEAT-051, FEAT-052, FEAT-053, FEAT-054, FEAT-055
- FEAT-064 (performance: memory optimization)

### P7 — Release Candidate
- FEAT-073 (security: config encryption)

---

## Feature Dependencies Graph

```
FEAT-001 (agent loop)
    ├── FEAT-004 (multi-turn context)
    ├── FEAT-002 (streaming) → FEAT-006 (cancellation)
    └── FEAT-003 (session resume) → FEAT-022 (crash recovery)

FEAT-010 (patch edits)
    ├── FEAT-040 (inline diff)
    ├── FEAT-013 (git commit)
    └── FEAT-070 (path sandboxing)

FEAT-011 (parallel tools)
    └── FEAT-050 (real subagent tools) → FEAT-051 (parallel agents)
                                            └── FEAT-052 (dynamic replanning)
                                                └── FEAT-053 (msg bus UI)

FEAT-030 (semantic search)
    ├── FEAT-031 (symbol context) → FEAT-032 (dependency planning)
    ├── FEAT-033 (incremental index)
    └── FEAT-034 (cache-aware search)

FEAT-012 (streaming output)
    └── FEAT-072 (expanded redaction)
```
