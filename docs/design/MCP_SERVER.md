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

### 4.5 `record_memory`

Records or updates an engineering memory entry (decision, constraint,
context). Arguments: `key` (stable identifier), `value`, optional `tags`,
`confidence`/`importance` (clamped to [0,1]), optional `source`.

Enforced: non-empty key/value, 64 KiB value bound, tags sorted +
de-duplicated, deterministic upsert id from the key, and **secret
redaction** through `redact_secrets_public` before anything touches
storage. Persists to `.codebro/engineering_memory.json`.

**Use:** let the agent persist what it learned so the next session is not
amnestic — the project's *memory write path*.

### 4.6 `delete_memory`

Deletes an engineering memory entry by its exact `key`. Rejects unknown
keys. Persists to `.codebro/engineering_memory.json`.

**Use:** remove stale or wrong entries; completes the memory lifecycle
(create/read/update/delete).

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

- [x] `codebro serve` starts over stdio and negotiates with an MCP client.
- [x] All tools above return deterministic, schema-valid responses.
- [x] `apply_change` refuses out-of-workspace paths, blind overwrites and
      stale content.
- [x] Works with **at least one** host agent (OpenCode 1.18 + agnes model —
      verified end-to-end, see §8).
- [x] Zero security regressions: no new secret-leak paths; existing
      redaction authority is respected (verified: secret stored as
      `[REDACTED]` on disk).
- [ ] Works with a second host agent (Claude Code).

---

## 7. Open questions / follow-ups

- Engineering-memory **recording from the agent loop**: `record_memory` is
  exposed; teaching the host agent *when* to record is prompt/agent work.
- Whether `apply_change` should grow plan-awareness arguments (planned file
  list, strict mode) for cross-file changes.
- Optional SSE/HTTP transport for remote use (deferred).
- Fact store size: 27k facts ≈ 35 MB JSON. Consider compacting or moving
  the store to SQLite (already a dependency).

---

## 8. Connecting OpenCode

Verified end-to-end with OpenCode 1.18 and the `agnes-2.5-flash` model.

### 8.1 Prerequisites

1. Build codebro: `cargo build --release`
2. (Optional but recommended) Populate facts:
   `codebro init` from the target workspace
3. Verify the runtime: `codebro doctor` — healthy or warnings only

### 8.2 Register the MCP server

```bash
opencode mcp add codebro -- \
  /path/to/codebro/target/release/codebro serve --root /path/to/workspace
```

Confirm it connected:

```bash
opencode mcp list
# ●  ✓ codebro  connected
```

### 8.3 Use it

```bash
cd /path/to/workspace
opencode
```

The tools appear to the agent prefixed with the server name:
`codebro_workspace_context`, `codebro_engineering_facts`,
`codebro_engineering_memory`, `codebro_apply_change`,
`codebro_record_memory`, `codebro_delete_memory`.

For a one-shot non-interactive run with all approvals auto-granted:

```bash
opencode run --auto "What is this workspace? Use codebro_workspace_context."
```

### 8.4 Verified behaviour (2026-08-15)

- Agent discovers and calls `codebro_workspace_context` and reports the
  fact counts (27,209 facts on the codebro repo itself).
- Agent calls `codebro_engineering_facts` filtered by kind.
- Agent records memory via `codebro_record_memory`; entry persisted and
  resolvable in later sessions.
- Agent attempts to create a file with a non-empty `old`; `apply_change`
  **refuses** with an actionable error; the agent self-corrects to
  `old=""` and the guarded change succeeds.
- All test artifacts cleaned afterwards; no secrets stored.

### 8.5 Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `opencode mcp list` shows server, tools missing in session | Restart the opencode session; MCP tools are loaded at session start |
| Tool call fails with `-32602` | Argument validation error — read the message; e.g. creating a file requires `old=""` |
| `未提供令牌` / auth errors | Provider API key missing in the environment (`AGNES_API_KEY` etc.) |
| `doctor` reports fact validation issues | `codebro init` again; deterministic rebuild keeps the store consistent |
