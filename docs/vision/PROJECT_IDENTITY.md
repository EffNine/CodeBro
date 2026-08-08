# Project Identity

**Version:** 1.1.0
**Status:** Active
**Date:** 2026-08-08

---

## Persistent Project Identity

Every project that CodeBro operates in has a persistent identity stored in `.codebro/`. This identity is what makes CodeBro remember — across sessions, across restarts, across days of work.

### What Is Persistent
| File | Purpose |
|------|---------|
| `.codebro/workspace.json` | Project metadata: language, framework, build system, active files |
| `.codebro/index.json` | Repository index: file list, sizes, languages, last-modified timestamps |
| `.codebro/memory.json` | Project memory: decisions, preferences, task history |
| `.codebro/project_memory.json` | Intelligence memory: important symbols, patterns, architecture |
| `.codebro/skills/` | Project-specific skills |
| `.codebro/traces/` | Operation traces: task history, tool usage, lessons learned |
| `.codebro/sessions/` | Session history: timelines, metrics, outcomes |

### Why This Matters
Without persistent project identity, CodeBro is a stateless chatbot that forgets everything between sessions. With it, CodeBro becomes a project-aware engineering intelligence runtime that gets smarter the longer it works on a codebase.

---

## Engineering Decisions

These are the non-negotiable engineering choices that define how CodeBro is built.

### Decision 1: Rust for the Core
**Choice:** CodeBro is written in Rust.
**Reason:** Memory safety without a garbage collector, zero-cost abstractions, deterministic performance, and a rich ecosystem for TUI (ratatui), async (tokio), and serialization (serde). The terminal engineering runtime pattern demands low latency and high reliability — Rust provides both.
**Tradeoff:** Longer development time, steeper learning curve for contributors unfamiliar with Rust.

### Decision 2: Trait-Based Abstraction
**Choice:** All major subsystems are abstracted behind traits (`Provider`, `Tool`, `SubAgent`, `PreferenceEngineTrait`, etc.).
**Reason:** Traits enable testability (mock providers, mock tools), extensibility (new providers without core changes), and modularity (subsystems can be swapped independently).
**Tradeoff:** Additional indirection; trait objects have a small runtime cost compared to monomorphized generics.

### Decision 3: Event-Driven Architecture
**Choice:** Communication between modules flows through an event system, not direct function calls.
**Reason:** Events decouple modules, enable logging and tracing of every interaction, and allow the TUI to react to runtime state changes without tight coupling.
**Tradeoff:** More complex data flow; debugging requires tracing event chains rather than call stacks.

### Decision 4: Diff-Based File Editing
**Choice:** CodeBro edits files using unified diffs (the patch engine), not by reading and rewriting entire files.
**Reason:** Diffs are human-readable, reversible, and enable preview-before-apply. They also make it possible to show the user exactly what will change before it changes.
**Tradeoff:** Diff generation is more complex than direct write; edge cases (empty files, binary files) require special handling.

### Decision 5: JSON for Structured State, TOML for Configuration
**Choice:** Configuration lives in TOML (`config.toml`). All structured state (memory, skills, preferences, traces) lives in JSON.
**Reason:** TOML is optimized for human-edited key-value configuration. JSON is optimized for programmatic serialization of complex nested structures. Using the right format for each job reduces friction in both domains.
**Tradeoff:** Two serialization formats to maintain; inconsistent file extensions in the config directory.

---

## Architecture Decisions

These are the documented decisions that shaped CodeBro's architecture. They are recorded as ADRs in `docs/ADR/`.

### Key ADRs
| ADR | Topic | Status |
|-----|-------|--------|
| ADR-001 | Provider Runtime Architecture — all LLM communication flows through the `Provider` trait | Accepted |
| ADR-002 | Tool Runtime Architecture — tools are dispatched through a registry, not hardcoded | Accepted |
| ADR-003 | Runtime State Machine — the agent loop is a state machine with explicit transitions | Accepted |
| ADR-004 | Reliability Layer — timeouts, retries, and error classification for all external calls | Accepted |
| ADR-005 | Tool Capability Model — tools declare their capabilities; the permission system uses these declarations | Accepted |
| ADR-006 | Tool Hook System — hooks allow pre/post execution logic without modifying tool code | Accepted |
| ADR-007 | Tool Lifecycle Management — tools are registered, discovered, and unloaded dynamically | Accepted |
| ADR-008 | Intelligence Platform Architecture — the adaptive platform sits on top of the frozen core | Accepted |
| ADR-009 | Configuration Versioning — semantic versioning with migration pipeline for config evolution | Accepted |

### Architecture Principles
1. **Core is frozen** — `agent/`, `tools/`, `providers/`, `config/` are stable modules. New functionality goes in `adaptive/` or as extensions.
2. **Modules communicate via events** — no direct cross-module function calls for state changes.
3. **Provider trait is the sole LLM interface** — no `reqwest` calls outside the provider module.
4. **TUI is display-only** — the TUI reads state and renders it; it does not mutate state directly.
5. **All persistence is human-readable** — no binary formats for user-facing data.

---

## Roadmap

### Completed (P0–P5)
- Core agent loop and TUI
- Provider abstraction (OpenAI, OpenRouter, DeepSeek, Ollama, LM Studio)
- Tool system (filesystem, shell, git, patch)
- Repository indexing and context building
- Session memory and advanced memory (short-term, project, global)
- Memory consolidation engine
- Skill system with lifecycle
- Permission safety layer
- Agent operation tracing
- Tree-sitter integration and symbol indexing
- Multi-agent architecture (research, planning, coding, testing, review)
- TUI command center with live agent monitoring
- Agent coordination layer with message bus

### In Progress (P6)
- Adaptive Developer Platform (Preference Engine, Intent Engine, Recommendation Engine)
- Trust Model and Learning Policy
- Cost Policy and Model Routing
- Approval Gate for all adaptive actions
- Profile Engine

### Planned (P7–P9)
- P7: Workflow Engine, Profile Engine, Subagent Orchestrator, Model Routing
- P8: MCP Lifecycle, Skill Lifecycle (full), Plugin SDK
- P9: Integration hardening, UI polish, performance optimization

---

## Current Sprint

The current sprint focuses on completing the P6 Adaptive Developer Platform. Key deliverables:

1. **Preference Engine** — Schema, validation, audit trail, TUI panel
2. **Intent Engine** — Rule-based classification, confidence scoring, disambiguation
3. **Recommendation Engine** — Unified recommendation format, ranking, deduplication
4. **Trust Model** — Multi-factor scoring, explanations, historical accuracy tracking
5. **Cost Policy** — Multi-level limits, cost tracking, override mechanism
6. **Learning Policy** — Allowed/forbidden learning categories, audit trail
7. **Approval Gate** — TUI integration for all adaptive recommendations

Each deliverable follows the Architecture Decision Record process before implementation begins.

---

## Vision

CodeBro will become the most trusted terminal engineering intelligence runtime for professional developers. It will be known for:

- **Reliability** — It never crashes, never loses work, never surprises.
- **Transparency** — Every action is visible, every decision is explainable.
- **Intelligence** — It understands codebases, not just task inputs.
- **Discipline** — It stays focused on engineering. It does not become a general-purpose assistant.
- **Extensibility** — The community can extend it without modifying the core.

The vision is not to build the most feature-rich AI coding tool. It is to build the most trustworthy one.

---

## Constraints

These constraints are non-negotiable. They exist to protect the product's identity.

### Hard Constraints
1. **No GUI** — CodeBro is a terminal application. No web dashboard, no desktop app, no Electron wrapper.
2. **No autonomous action** — No file write, no shell command, no model switch, no configuration change happens without explicit or implicit user approval.
3. **No hidden state** — All state is persisted to human-readable files. Nothing exists only in memory.
4. **No network calls outside the provider** — All LLM communication goes through the `Provider` trait.
5. **No feature that replaces the developer's judgment** — CodeBro proposes; the developer decides.

### Soft Constraints
1. **Prefer Rust standard library over external crates** — Fewer dependencies mean fewer security and maintenance issues.
2. **Prefer composition over inheritance** — Traits and structs compose; inheritance creates fragile hierarchies.
3. **Prefer explicit over implicit** — Hidden behavior erodes trust. Make everything visible.
4. **Prefer small, focused modules** — A module with one responsibility is easier to test, document, and maintain.

---

## Why Project Memory Is More Important Than Task Memory

Task memory is the record of what was said during a task. Project memory is the record of what was learned about the codebase.

Task memory is useful for continuity within a session. Project memory is useful for continuity across sessions, across days, across the lifetime of the project.

Consider two scenarios:

**Scenario A — Task memory only:**
The user asks CodeBro to add authentication. CodeBro researches the codebase, implements the change, and the session ends. The next day, the user asks CodeBro to add OAuth. CodeBro has no memory of the previous authentication implementation. It starts from scratch, re-reads the same files, re-discovers the same patterns.

**Scenario B — Project memory:**
The user asks CodeBro to add authentication. CodeBro researches, implements, and stores the lesson: "Authentication is handled in `src/auth/middleware.rs` using JWT tokens. The project uses `actix-web`." The next day, the user asks CodeBro to add OAuth. CodeBro retrieves the project memory, understands that authentication already exists, and builds on top of it rather than starting from scratch.

Scenario B is what makes CodeBro valuable over time. Scenario A is what every other AI coding tool does.

Project memory includes:
- Important symbols and their roles
- Architecture patterns discovered in the codebase
- Coding conventions (naming, error handling, testing)
- Decision history (why a certain approach was chosen)
- Failed attempts and what went wrong

Task memory includes:
- The task input transcript
- Tool calls made during the task
- The plan that was followed

Both are stored. But project memory is the differentiator. Task memory is a log. Project memory is intelligence.
