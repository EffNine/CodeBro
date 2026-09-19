# P2 Implementation — Sessions + History + Recall

Date: 2026-09-06. Status: **implemented, tested, verified** (109
context-runtime lib tests + 336 mcp-server lib tests green,
clippy/fmt/dep-script clean — see §18).
Scope: `docs/evolution/PHASE4_PLAN.md` P2 row, built on the P0 foundation
and the P1 identity substrate (`task_id` + task-aware search/count).

P2 answers the second question and stops there:

```text
P1  What has the user explicitly told CodeBro?
P2  What happened during previous work?          ← this document
P3  What should CodeBro learn from what happened? (not built)
```

A fresh OpenCode session can now ask CodeBro what happened during previous
engineering work and receive relevant, scoped, provenance-preserving
evidence — without the user manually managing history.

## 1. Architecture

Extension over replacement, again. P0's `events` table and reserved
`sessions` table already had the history shape; P1's `task_id` already had
the task identity. P2 adds the missing session/history/recall pieces
around them:

```text
context-runtime (new modules, same crate, still → core only)
├── types.rs      EventRecord + task_id/summary/dedup_key/source (additive,
│                 serde-defaulted); strict bounds for the new fields
├── history.rs    SessionStatus/SessionRecord, HistoryKind taxonomy,
│                 HistoryInput/OpenSession/SessionFilter, session lifecycle
│                 + record_history + rebuild_history_fts (impl ContextStore)
├── recall.rs     RecallScope/RecallQuery/RecallHit/RecallGroup/RecallOutcome,
│                 excerpt_of, ContextStore::recall
├── db.rs         SCHEMA_VERSION 3, stepwise migrate (v3 step with
│                 probe-first resume + history backfill)
└── store.rs      event columns extended, append_event syncs history FTS
                  in-txn, shared sync_history_fts/EVENT_COLUMNS helpers

mcp-server (thin adapter + passive capture, composition untouched)
├── recall tool (tool 21): capability-level query interface, read-only
├── history_capture.rs: best-effort passive capture helper (never fails tools)
├── passive wiring: remember/forget → decision+observation,
│   apply_change(s) → change_applied, sandbox_test/build → validation
└── server instructions advertise the recall flow
```

No schema redesign, no second event abstraction, no new store. The
`context` packet is byte-identical in shape (history is query-driven,
never dumped).

## 2. Session model

`SessionRecord`: opaque `ses::<16 hex>` id (time+pid+counter mix,
existence-checked; never a workspace path or row id), canonical
`workspace_root`, optional `task_id`, optional `title`/`source`/
`parent_session_id`, `status` (active/completed/abandoned),
`started_at`/`updated_at`/`ended_at`, `end_reason`, cached `event_count`.

Deliberately narrower than the Hermes 40-column shape: no model/config
metadata (CodeBro never selects models), no idle-gap auto-clustering
(sessions are explicit + resumable, not inferred).

## 3. Session lifecycle

`ACTIVE → COMPLETED | ABANDONED`, enforced at the store:

- Only `Active` sessions close; double-close is refused (history never
  rewrites an ending); closing requires a terminal status.
- A crash leaves `Active` rows behind. They are reported as **stale**
  (derived at read time, `updated_at` older than 24 h), never
  auto-completed — an interrupted session "did not complete
  successfully" and says so. Recall flags stale sessions; they stay
  reopenable via `ensure_active_session`.
- Restart safety: `ensure_active_session(workspace, task, …)` resumes the
  newest active session instead of duplicating it; closing then ensuring
  opens fresh (no resurrection). Tested across real handle reopen.

## 4. History model

One event abstraction: `EventRecord` extended with four additive optional
fields (`task_id`, `summary`, `dedup_key`, `source`). The P2 write seam is
`record_history(HistoryInput)`, which in one transaction: canonicalizes
the workspace, redacts secrets from summary+payload **before** storage
(canonical row and FTS alike), truncates oversized text with a marker
(history capture must not fail on a long tool result — `append_event`
keeps its P0 refuse policy for callers that want a gate), enforces
dedup keys, links + touches the session (event count + heartbeat), and
syncs the derived FTS index.

Closed taxonomy (`HistoryKind`, 11 kinds): session_started/ended,
user_message, assistant_message, tool_execution, tool_result, decision,
change_applied, validation, error, observation. Structural capture only —
kinds describe what happened, never what it means.

History is append-only: events are never updated, superseded, or
retired. `forget` retires *records*, never history. Ordering is always
(`created_at`, `id`) — explicit timestamps, never insertion order.

## 5. Passive capture

`history_capture::capture()` — best-effort (errors debug-logged, never
fail the tool), summaries not transcripts, session-linked via
`ensure_active_session`, source-tagged `mcp-passive`:

| Operation | Kind | Summary |
|---|---|---|
| `remember` | decision | `recorded {kind} {namespace}: {160-char excerpt}` |
| `forget` | observation | `retired context record {id} ({action})` |
| `apply_change(s)` | change_applied | paths only, never contents |
| `sandbox_test/build` | validation | `{command} → {classification} (verified: …)` + 500-char summary |

`recall`, `context`, and all reads capture nothing (no recursion).
Memory/identity writes, consults, and raw execs stay quiet by design —
documented in the module so an absent event is explicable, not mysterious.

## 6. FTS architecture

`events_fts` FTS5 (`summary`, `text` = 2k payload excerpt, `kind`,
`tool`, `outcome`, `event_id UNINDEXED`, unicode61) is a **derived**
index: canonical `events` rows are authoritative. Every write path syncs
in the same transaction (`append_event` and `record_history`); rebuild
from canonical rows is one call (`rebuild_history_fts`, returns count).
OR candidacy for natural questions (AND would refuse every question
carrying stopwords); BM25 + deterministic priors restore precision.

## 7. Recall

`ContextStore::recall(RecallQuery)`: FTS candidates → canonical
workspace/task filtering (**before** ranking, re-checked in Rust as
defense in depth) → deterministic ranking → session grouping (≤3
excerpts/session, ≤limit total) → bounded excerpts (240 chars + marker).

Actual ranking, stated plainly: BM25 → task match → kind importance
(decision 100 … observation 20) → recency → id. BM25 does not understand
intent; this is lexical relevance plus structural priors. `total_matches`
counts bounded candidates (≤500), not the table — recall is evidence,
not analytics.

Output per hit: event id, kind, unix timestamp, derived scope
(task/project/global), task/tool/path/outcome/source, stale flag,
excerpt. Per group: session id/title/status/stale, workspace, task,
session total. Provenance label: `historical-evidence`.

## 8. Isolation

- Workspace: P1 canonicalization reused everywhere (no second mechanism);
  equivalent representations share one history (tested).
- Task: task-scoped events invisible without the exact task id — in
  recall (all scopes) and in ranking boosts. Task B recall returns zero
  Task A rows (tested at store and MCP levels).
- Global: explicit opt-in only (`scope=global`), every hit tagged with
  its workspace. Default is project. Cross-project FTS bypass tested:
  FTS physically contains A's row, B's recall returns zero.
- Session listing is inventory (includes task sessions); recall is the
  enforcement point.

## 9. Security / bounds

- `redact_secrets_public` (the single core authority) runs before
  truncation on every history write; tests prove secrets absent from
  canonical rows, FTS, and recall excerpts.
- Summary ≤2000 chars, payload ≤8192 bytes (truncated + marked, never
  refused on the history path); dedup/source/task bounded; recall
  excerpts ≤240 chars; MCP envelope 256 KiB; binary/garbage payloads
  stored safely and stay searchable-valid.
- Oversized/blank identities (workspace/task/dedup) refused with reasons.

## 10. MCP surface

One new tool (21 total), no table CRUD:

- **`recall`** — `query` (required, ≥1 searchable token), `scope?`
  (project default | task + `task_id?` | global opt-in), `kinds?`
  (parsed taxonomy, invalid rejected), `session_id?`, `limit?`
  (default 10, max 50), `workspace_root?`. Read-only: no mutation
  lock, no history capture. Validation errors are `invalid_params`.

## 11. Context integration

Unchanged shape. `context` keeps serving fingerprint/intent/project
evidence; history never enters the packet (asserted: no `history`/
`recall` keys). Flow: current task → context → OpenCode reasons →
recall on demand → OpenCode reasons again. CodeBro supplies evidence,
never decisions.

## 12. Migrations

v3 step, probe-first resume (like v2): sessions +=
task_id/status/source/parent_session_id/updated_at (backfill
`status='active'`, `updated_at=opened_at`); events +=
task_id/summary/dedup_key/source (+ task index, partial-unique dedup
index); `events_fts` created + backfilled from stored (already-redacted)
text. Tested: fresh, v1→v3 (rows + both FTS indexes intact), v2→v3,
partial-v3 resume, restart-after-migration (via reopen tests).

## 13. Concurrency / idempotency

- In-process serialization via the store mutex (8×25 history storm +
  reads-during-writes green); two-connection writers (WAL + busy
  timeout) lose nothing; restart after concurrent writes durable.
- Cross-process *forked* writer test not added (same standing as the P0
  gap): two handles over one file is the practical approximation.
- Idempotency: `dedup_key` partial-unique; replay returns the original
  id with `duplicate=true` (tested). Key-less events are always distinct.

## 14. P3 boundary (documented, not built)

Preserved for learning: kinds, outcomes, task/project/session links,
timestamps, sources, validation results, failure text. Not built: no
learning candidates, no inference, no auto-promotion, no embeddings, no
summarization, no skill/task runtime. `Experience` records remain
storable-but-unminted, as in P1.

## 15. Deviations from the mission brief (with reasons)

1. **OR instead of AND candidacy** (§6): AND over FTS tokens refuses
   natural questions ("why are we using SQLite again?" contains
   why/are/using). Recall is question-driven, unlike keyword record
   search which keeps AND. Ranking restores precision.
2. **`total_matches` counts bounded candidates** (§7), not table rows —
   an exact count would require unbounded scans; recall promises
   evidence, not analytics.
3. **No `session_create`/`event_insert` MCP tools**: sessions are
   managed passively (`ensure_active_session`) + at the store layer.
   The MCP surface stays one capability (`recall`), per the brief's
   preference.
4. **`limit: 0` means default 10** (not clamp-to-1): friendlier for
   omitted MCP args; documented on the query type.

## 16. Known limitations (not defects)

- Passive capture covers six operations (§5); raw execs, consults, and
  memory/identity writes are quiet. OpenCode chat text itself is never
  captured (CodeBro behind MCP cannot see it) — `user_message`/
  `assistant_message` kinds exist for future bridging, nothing mints
  them yet.
- No explicit MCP session-close: long-lived workspace sessions stay
  `Active` until closed at the store layer; staleness is the honest
  signal, not auto-completion.
- BM25 ranking is lexical; kind importance is a fixed table, not learned.
- redact_secrets recompiles regexes per call (shared P0 mechanism):
  bulk history seeding is slowish (~15 s / 1000 events in test); recall
  itself is fast (<5 s / 1000-event recall asserted).
- No fork-based cross-process writer test (P0 gap, still open).
- P2 test-hermeticity incident (found and fixed during implementation):
  passive capture made sandbox/apply/test paths write to the user-context
  store, so test servers using the default state dir (`local_sandbox_server`,
  multi-workspace `::new`, spawned `serve` children) wrote test events into
  the developer's real `~/.codebro/state.db`, and parallel binaries racing
  one file triggered three quarantine-and-recreate cycles. Forensics proved
  all quarantined content was test junk (zero user records); the artifacts
  were removed. Fix: every test server uses an explicit state dir
  (`with_state_dir` into an owned TempDir; `CODEBRO_STATE_DIR` on spawned
  children). The full suite now runs with zero `~/.codebro` changes
  (asserted by before/after directory diff).

## 17. Tests added

context-runtime 75 → 109 (+34): sessions (lifecycle, uniqueness,
resume-no-duplicate, stale, counts, parents), history (linking, order,
dedup, truncation, secrets, garbage, canonicalization, concurrency,
two-connection), recall (relevant, empty-refused, no-result, grouping
caps, task boost/scope, cross-project, kind filter, excerpts,
scope-identities, rebuild, FTS-corruption recovery, FTS-bypass,
stale-flag, session filter, 1000-event performance), migration (fresh
v3, v1→v3, v2→v3, partial resume).

mcp-server lib 329 → 336 (+7): decision recallable end-to-end, change
recallable, project isolation, task isolation, no-recursion + arg
rejections, context-history separation, boundedness + provenance.
Plus `recall` arms in the tool-description/router regression lists and
the test-helper dispatcher.

## 18. Verification

```text
cargo test --workspace        # all green, 0 failed
cargo clippy --workspace --all-targets   # 0 warnings
cargo fmt --check             # clean
scripts/check_workspace_deps.sh          # OK (no new deps: history/recall → core only via context-runtime)
```

P1 numbers are superseded by the run above (re-run at completion per
the brief; counts grew monotonically, nothing regressed).

## 19. Product test (brief §22)

- Session 1 (SQLite decision via remember) → Session 2 ("Why are we
  using SQLite again?" via recall) returns the decision with session +
  scope + source: covered by `p2_remembered_decision_is_recallable`.
- "Have we tried this before?" → previous attempt + outcome, no "never
  use X" inference: recall returns events with outcomes; P2 infers
  nothing (no promotion path exists in code).
- Project isolation: `p2_recall_enforces_project_isolation` (0 rows for B).

## 20. Verdict

**COMPLETE.** A fresh OpenCode session can ask CodeBro what happened
during previous engineering work and receive relevant, scoped,
provenance-preserving evidence without managing history. P3 (learning +
inference) is unblocked and unimplemented — as required.
