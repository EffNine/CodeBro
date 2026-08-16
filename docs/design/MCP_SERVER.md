# MCP Server — Engineering Runtime Interface

**Document:** `docs/design/MCP_SERVER.md`
**Version:** 1.0.0
**Phase:** P7 — MCP Server
**Status:** Implemented
**Date:** 2026-08-15
**Owner:** CodeBro Engineering

---

## 1. Purpose

CodeBro exposes its engineering context layer as a **Model Context Protocol
(MCP) server** so that battle-tested agent frontends — Claude Code, OpenCode,
Codex, Cursor, Goose — can act as the interface while CodeBro owns project
truth and persistent engineering context (plus an optional guarded mutation
API for controlled workflows).

The legacy chat TUI was moved to the `tui-legacy` branch and is out of scope
for this design.

---

## 2. Positioning

CodeBro is an **engineering context & memory layer**. It is **not** "another
bag of MCP tools" (filesystem / shell / git / memory) — those are commodity
capabilities the host agent already provides.

The differentiated surfaces are:

- **Project identity** — the agent knows *what project it is in*, its
  language, frameworks, build system, constraints and architecture — without
  re-discovering it per session.
- **Engineering facts** — verified, provenance-carrying facts (modules,
  packages, symbols, tests, dependencies, architecture rules) retrieved
  semantically with locations.
- **Engineering memory** — recorded decisions, constraints and prior context
  resolved by task relevance with confidence, source and tags.
- **Guarded change application** *(optional)* — mutations routed through the
  change engine for controlled/autonomous workflows: workspace boundary, plan
  awareness, stale-content protection, audit. Agents use their native editing
  tools for normal edits; `apply_change` is not mandatory.

The host agent owns: model, conversation, agent loop, planning, tool
selection, UX, execution strategy.
CodeBro owns: project truth, engineering state, persistent context, guarded
mutations (optional), policy, verification boundaries.

### 2.1 Trust model: verified facts vs agent-recorded memory

CodeBro deliberately keeps **three distinct classes of information** and the
MCP surfaces never blur them:

| Class | Source | Trust | Surface |
|---|---|---|---|
| **Verified facts** | `codebro init` (tree-sitter scan of real source) | High — deterministic, provenance-carrying, validated (0-issue store) | `engineering_facts`, `workspace_context` |
| **Engineering decisions** | Human-authored identity/constraints | Medium-high — declared intent | `workspace_context` (identity), `record_memory` with explicit source |
| **Agent-recorded context** | Agents calling `record_memory` | **Low — unverified beliefs** with self-declared confidence | `engineering_memory` (confidence score shown), `memory_stats` |

Consequences:

- `engineering_memory` content is **agent-recorded context, not verified
  engineering truth**. Responses carry the self-declared `confidence`, plus
  `source`/`tags` provenance so the host agent can judge trustworthiness.
- `record_memory` accepts arbitrary agent text; it is **never** promoted to
  the verified fact store. There is no promotion path in this phase.
- Server instructions tell agents to treat memory as contextual, and the
  fact store as verified.
- This phase does **not** introduce a governance system; the distinction is
  preserved structurally (separate stores, separate tools, provenance on
  memory) and documented here.

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

Transport: **stdio** (one server process per host agent session). Each tool
call constructs a fresh view of the runtime from the workspace root. The
immutable fact store is cached per process (with an mtime check so a
concurrent `codebro init` is picked up), avoiding a 20+ MB JSON re-parse on
every call (~8× faster steady-state).

---

## 4. Tools

### 4.1 `workspace_context`

Returns project identity (name, languages, frameworks, build system,
constraints, architecture summary), the workspace root and current fact-store
counts.

**Use:** orient the agent at session start.

### 4.2 `engineering_facts`

**Relevance-ranked fact retrieval** over the verified fact store
(deterministic lexical matching — not embedding/vector search). Arguments:

- `query` (required): symbol/module/test name, name fragment, or path
  fragment, matched case-insensitively.
- `kind` (optional): filter by fact kind.
- `path` (optional): path substring filter.
- `limit` (optional): defaults to 10, capped at 50.

Returns **actual fact records** (kind, name, path, line, summary,
provenance) ranked deterministically — not raw ids. Matching: exact name >
name prefix > name substring > path substring > signature substring.

**Use:** "where is X defined", "what functions/structs exist", "which
module owns Y". This replaces the earlier id/count-only behavior which
caused agents to fall back to their own Glob/Grep/Read tools.

### 4.3 `engineering_memory`

Resolves engineering memory entries for a task query
(`task_keywords`, `active_file_tags`), ranked deterministically with
confidence scores, budget enforcement, and enriched provenance (`source`,
`tags`) projected from the persisted snapshot.

**Use:** "what decisions constrained this area before", "how was this
implemented previously". Content is agent-recorded context — see §2.1
trust model; never treat it as verified truth.

### 4.3b `memory_stats`

Read-only statistics about the engineering memory store: `entry_count`,
`total_budget`, `entries_with_source`, `avg_confidence`,
`oldest/newest_created_at`, and tag distribution.

**Use:** judge whether engineering memory holds meaningful state before
relying on it (e.g. "is memory empty or fresh?").

### 4.4 `apply_change` *(optional)*

Applies a guarded single-file change through the change engine. Arguments:
`path`, `old` (exact existing text; empty to create), `new`.

Enforced (and only these, matching the implementation):
- workspace path boundary (lexical `..`/absolute check **and** symlink-escape
  canonicalization)
- no blind overwrite: `old` must be non-empty for existing files
- unique replacement: an `old` text occurring more than once is rejected as
  ambiguous
- stale-content protection between prepare and apply

The engine is plan-less and non-strict by design in this phase — no plan
adherence is claimed because no plan is supplied.

**Use:** optional guarded mutation API for controlled/autonomous workflows.
Host agents use their native editing tools for normal coding edits — see §2
positioning. There is exactly one mutation path: `apply_change` routes
exclusively through `ChangeEngine::prepare` → `apply`; the MCP layer contains
no raw filesystem writes.

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

## 5. Explicit non-goals

- **No arbitrary shell execution** through CodeBro. Host agents already own
  shell; CodeBro must not become "another shell wrapper".
- **No filesystem/shell/git read tools** — commodity capabilities already
  present in every host agent. (Note: `engineering_facts` is a *fact*
  search over the verified store, not a source-file search tool.)
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

## 6.1 Security properties (verified by tests)

| Property | Mechanism | Verified |
|---|---|---|
| Path traversal (`..`, absolute outside root) | lexical component check + `starts_with(root)` | ✗ rejected |
| **Symlink escape** (link inside root → outside) | canonicalize target (or nearest existing ancestor for create paths) and compare against canonicalized root; macOS `/var`→`/private/var` handled | ✗ rejected, external target untouched |
| Blind overwrite | unique non-empty `old` match required | ✗ rejected |
| Stale content between prepare/apply | snapshot comparison at apply time | ✗ rejected |
| Secret leakage to disk | `redact_secrets_public` authority: `sk-`/`ghp_`/`glpat`/`xox`/`bearer`/`api_key`/`password=`/`token`/URL-credentials/`--flag` styles | ✗ `[REDACTED]` on disk |
| Memory write bounds | key ≤256 chars, value ≤64 KiB, ≤32 tags of ≤64 chars, confidence/importance clamped | ✗ rejected |
| Fact query bounds | limit clamped 1..50, empty query rejected (or must carry a kind/path filter) | ✗ rejected |
| Determinism | fact search + memory resolution sort stably | ✓ |

The change engine is the **only** mutation path (`apply_change` routes
exclusively through `ChangeEngine::prepare → apply`; no raw writes in the
MCP layer), and `record_memory` is the only memory write surface.

---

## 7. Open questions / follow-ups

- Engineering-memory **recording from the agent loop**: `record_memory` is
  exposed; teaching the host agent *when* to record is prompt/agent work.
- Whether `apply_change` should grow plan-awareness arguments (planned file
  list, strict mode) for cross-file changes.
- Optional SSE/HTTP transport for remote use (deferred).
- Fact store size: ~24 MB JSON for ~13.6k facts. Consider compacting or
  moving the store to SQLite (already a dependency).

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
  fact counts (14,470 facts on the codebro repo itself, after dedup
  fixes — see §9).
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

---

## 9. A/B comparison: with vs without CodeBro

Methodology (2026-08-15): the same prompt, the same model
(`agnes-2.5-flash` in OpenCode 1.18.16), **separate fresh sessions**, on
the codebro repo itself. The "without" session ran with the codebro MCP
server removed from the OpenCode global config; the "with" session had it
registered. Both sessions auto-approved permissions.

### 9.1 Test 1 — qualitative (architecture questions)

Prompt: *"Where is the guarded file-mutation prepare/apply seam? Is there
workspace-boundary enforcement? Which module owns the canonical mutation
path?"*

| | ✅ With CodeBro | ❌ Without CodeBro |
|---|---|---|
| File identified | `src/coding/permissions.rs` | `src/coding/permissions.rs` |
| Key struct | `ChangeEngine` (line 202) | `ChangeEngine` (line 202) |
| Boundary enforcement | `resolve_path` (line 491) | `resolve_path` (line 491) |
| Method used | grep/read (MCP tools **not** preferred) | grep/read |

**Conclusion:** for plain "where is X" questions, a capable agent answers
equally well with grep — CodeBro adds no measurable value here.

### 9.2 Test 2 — quantitative (whole-project facts)

Prompt: *"Report the total number of symbols, tests, and modules, and give
3 example symbol ids."* Ground truth comes from the facts store.

| Metric | ✅ With CodeBro | ❌ Without CodeBro | Ground truth |
|---|---|---|---|
| Symbols | **10,514** (exact, from facts store) | 4,227 ❌ (counted only `pub fn/struct/enum` — missed private items and methods) | 10,514 |
| Tests | **3,602** (exact) | 2,799 ❌ (counted only `#[test]` attrs — missed helpers) | 3,602 |
| Modules | **351** (exact) | 559 ❌ (counted `mod` declarations, not source files — wrong definition) | 351 |
| Example symbol ids | Real ids from the store | **Constructed from a guessed pattern** ⚠️ (agent invented `sym::…::FactId::new@142` from the id format, not from data) | store ids |
| Tool calls | 1 MCP call (`codebro_workspace_context`) | 5+ grep/glob (missed the target) | — |

**Conclusion:** for whole-project factual questions, CodeBro is
materially better: exact counts vs under-counts, and real identifiers vs
invented ones (hallucination risk). This is the differentiating scenario
the product targets.

### 9.3 Key findings

1. **Quantitative questions expose CodeBro's value; qualitative ones do
   not.** An agent can grep its way to "where is the seam" but cannot
   cheaply and correctly answer "how many symbols / list real ids" — it
   under-counts or fabricates. CodeBro's facts store answers exactly.
2. **Agents do not auto-prefer MCP tools.** In Test 1 the agent with
   CodeBro available still chose grep. Value only materialised when the
   prompt instructed the tool. Implication: strengthen the server's
   `instructions` field and/or prompt guidance so agents consult CodeBro
   for project facts by default.
3. **`codebro doctor` and the MCP server agree** (14,470 facts, 0
   validation issues), confirming the store is the single source of truth.

### 9.4 Follow-ups from this test

- Make `ServerInfo.instructions` more directive: "for any question about
  project-wide symbols, modules, tests or decisions, prefer
  `codebro_engineering_facts` / `codebro_workspace_context` before
  grepping."
- Consider a benchmark harness (see advisor's 90-day plan: measure
  context-error reduction across ≥10 tasks).

---

## 10. Auto-detection verification (2026-08-15)

After the directive `ServerInfo.instructions` landed (see §9.4 → done),
re-ran the scenarios **without any mention of codebro in the prompts** to
verify the agent consults CodeBro by itself.

### 10.1 Cross-module impact map

Prompt: *"I'm about to modify the change engine. Which modules reference
ChangeEngine? Give me a map of what I'd be touching."*

Result — exact dependency map: `src/coding/permissions.rs` (owner),
`src/coding/mod.rs` (re-export), `src/mcp/mod.rs:216` (the **only external
consumer**), plus a per-file symbol count of the whole `coding` module
(208 symbols across 6 files). No other module imports `ChangeEngine`.

### 10.2 Change planning

Prompt: *"I want to add a new MCP tool `check_memory_count`. Produce a
precise change plan."*

Tool calls (agent-initiated): `codebro_workspace_context` →
`codebro_engineering_memory` → `codebro_engineering_facts(symbol)`.

Result: a correct, minimal plan — **one file** (`src/mcp/mod.rs`), a
read-only tool following the existing `#[tool]` pattern, tolerant
`memory.load()`, `entry_count()` on the snapshot. No speculative edits.

### 10.3 Memory recording

Prompt: *"I want this constraint remembered: all file mutations must go
through ChangeEngine, never raw writes."*

Agent called `codebro_record_memory` itself with rich metadata:
confidence 0.95, importance 0.9, tags `[architecture, change-engine,
constraint, file-mutations]`, source `colleague-informed`. Entry verified
persisted to `.codebro/engineering_memory.json`.

### 10.4 Anti-amnesia (fresh session)

Prompt (in a **new, empty session**): *"Is there anything in this
project's recorded engineering context I should know before writing
files?"*

Agent called `codebro_engineering_memory` first, **retrieved the
constraint recorded in the previous session**, and connected it to
action: "use `codebro_apply_change` rather than raw writes". This is the
persistent-context scenario the product targets: knowledge survives
across sessions without being re-stated.

### 10.5 Conclusion

- **Auto-detection works**: in all four scenarios the agent consulted
  CodeBro tools before (or alongside) grep — no prompt hints required.
- **Memory lifecycle is closed**: record (10.3) → persist → retrieve in a
  fresh session (10.4) — the anti-amnesia loop is functional end-to-end.
- Facts store used by the agent grew with dependency facts (33
  dependencies from Cargo.toml, validation still 0 issues). Dependency
  discovery since also supports Go `go.mod` (verified on Conductor:
  39 deps — 8 direct, 31 transitive).
