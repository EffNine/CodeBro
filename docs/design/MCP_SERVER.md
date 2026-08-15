# MCP Server — Engineering Runtime Interface

**Document:** `docs/design/MCP_SERVER.md`
**Version:** 1.0.0
**Phase:** P7 — MCP Server
**Status:** Proposed
**Date:** 2026-08-15
**Owner:** CodeBro Engineering

---

## 1. Purpose

CodeBro exposes its engineering runtime as a **Model Context Protocol (MCP)
server** so that battle-tested agent frontends — Claude Code, OpenCode, Codex,
Cursor, Goose — can act as the interface while CodeBro owns project truth,
persistent engineering context and guarded mutations.

The TUI is **frozen** (not deleted): it remains one possible client, but is no
longer the strategic center of the product.

---

## 2. Positioning

CodeBro is **not** "another bag of MCP tools" (filesystem / shell / git /
memory). Those are commodity capabilities the host agent already provides.

The differentiated surfaces are:

- **Project identity** — the agent knows *what project it is in*, its
  language, frameworks, build system, constraints and architecture — without
  re-discovering it per session.
- **Engineering facts** — verified, provenance-carrying facts (modules,
  packages, symbols, tests, dependencies, architecture rules) queried
  deterministically.
- **Engineering memory** — recorded decisions, constraints and prior context
  resolved by task relevance with confidence scoring.
- **Guarded change application** — mutations routed through the change engine:
  workspace boundary, plan awareness, stale-content protection, audit. No
  blind overwrites.

The host agent owns: model, conversation, agent loop, planning, tool
selection, UX, execution strategy.
CodeBro owns: project truth, engineering state, persistent context, guarded
mutations, policy, verification boundaries.

---

## 3. Architecture

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

Transport: **stdio** (one server process per host agent session). The server
is stateless between calls: each tool call constructs a fresh view of the
runtime from the workspace root.

---

## 4. Tools

### 4.1 `workspace_context`

Returns project identity (name, languages, frameworks, build system,
constraints, architecture summary), the workspace root and current fact-store
counts.

**Use:** orient the agent at session start.

### 4.2 `engineering_facts`

Queries the verified fact store, optionally filtered by kind
(`workspace | module | package | symbol | test | build_target | dependency |
relationship | reference | diagnostic | architecture_rule`).

Returns per-kind counts and matching fact ids.

**Use:** answer "what modules/packages/symbols exist", "which architecture
rules constrain this change".

### 4.3 `engineering_memory`

Resolves engineering memory entries for a task query
(`task_keywords`, `active_file_tags`), ranked deterministically with
confidence scores and budget enforcement.

**Use:** "what decisions constrained this area before", "how was this
implemented previously".

### 4.4 `apply_change`

Applies a guarded single-file change through the change engine. Arguments:
`path`, `old` (exact existing text; empty to create), `new`.

Enforced: workspace path boundary, no blind overwrite, unique non-empty `old`
match, stale-content refusal between prepare and apply.

**Use:** the only mutation surface CodeBro offers initially — deliberately
narrower than the host agent's own shell/filesystem tools, to prove
*safety and context* rather than raw capability.

---

## 5. Explicit non-goals (initial scope)

- **No arbitrary shell execution** through CodeBro. Host agents already own
  shell; CodeBro must not become "another shell wrapper".
- **No read/search/git tools initially** — commodity capabilities already
  present in every host agent.
- **No MCP *client* support yet** (the P6 `MCP_LIFECYCLE` direction is
  deferred until the server proves valuable).
- **No resource/prompt surfaces yet** — tools only, minimal surface.

---

## 6. MVP acceptance criteria

- [ ] `codebro serve` starts over stdio and negotiates with an MCP client.
- [ ] All four tools above return deterministic, schema-valid responses.
- [ ] `apply_change` refuses out-of-workspace paths, blind overwrites and
      stale content.
- [ ] Works with **at least two** host agents (OpenCode, Claude Code).
- [ ] Zero security regressions: no new secret-leak paths; existing
      redaction authority is respected.

---

## 7. Open questions / follow-ups

- Fact-store **population pipeline** (`codebro init`: static analysis →
  `FactsModel` → `.codebro/facts.json`). The server reads facts if present;
  population is the next milestone.
- Whether `apply_change` should grow plan-awareness arguments (planned file
  list, strict mode) for cross-file changes.
- Optional SSE/HTTP transport for remote use (deferred).
