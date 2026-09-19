# CodeBro P9 Implementation — Engineering Outcome & Feedback Loop

**Status:** COMPLETE (additive over frozen P0–P8; no P0–P8 behavior changed except the hardened open path, §16).
**Schema:** v7 (unchanged — no new tables, no migration).
**MCP tools:** 25 (unchanged — P9 adds NO tool; one new `task` action).
**Tests:** 1454/1454 (1426 baseline + 28 new), clippy `-D warnings` clean, `cargo fmt --check` clean.
**Architecture:** OpenCode does the work; CodeBro remembers what the work taught us — through an explicit, bounded, provenance-aware outcome-ingestion seam feeding the existing history/learning/recall/brief machinery.

---

## 1. Mission

P0–P8 built the engineering context/runtime core and the OpenCode
integration contract. What was missing was the return path:

```text
Task → Context → OpenCode reasoning → OpenCode implementation
  → Outcome → Evaluation → Learning Candidate → Persist / Reject / Defer
  → Future Context
```

P9 answers: *"After OpenCode completes an engineering task, how can
CodeBro capture the meaningful engineering outcome as durable,
trustworthy evidence without becoming the agent that performed the
work?"*

The answer is deliberately thin: one new `task` action (`outcome`)
that records structured outcome evidence as task-bound history, plus
hardening of the concurrent-open path the new workload exposed. Every
other P9 requirement — evaluation, learning candidacy, trust gates,
retrieval, brief projection — already existed and is reused unchanged.

## 2. Architecture map (reconnaissance findings)

The pre-implementation survey established that P9 is a wiring problem,
not a modeling problem. Every concept P9 needs already exists:

| P9 need | Existing owner | Location |
|---|---|---|
| Durable task identity + lifecycle | P5 task runtime (`tasks`, `task_checkpoints`) | `context-runtime/tasks.rs` |
| Bounded resume state | P5 checkpoints (immutable, versioned) | `context-runtime/tasks.rs` |
| Append-only evidence log + FTS | P2 history (`events`, `events_fts`) | `context-runtime/history.rs`, `store.rs` |
| Scoped keyword retrieval | P2 recall (FTS5 + importance + recency, deterministic) | `context-runtime/recall.rs` |
| Cautious inference (≥3 support, contradiction-aware, never self-confirmed) | P3 learning (`learning_candidates`, `AI_INFERRED`) | `context-runtime/learning.rs` |
| Authority vocabulary + evidence rules | P0 types (`Authority`, store-enforced citation) | `context-runtime/types.rs`, `store.rs` |
| Decision-neutral briefs | P7 `engineering_brief::assemble()` (read-only) | `mcp-server/engineering_brief.rs` |
| 25-tool agent contract | P8 `integration::contract` | `mcp-server/integration.rs` |
| Workspace authorization (closed) | P8 `WorkspaceRegistry` (exact-root, frozen) | `mcp-server/workspace_registry.rs` |
| Secret redaction authority | `redact_secrets_public` at every write seam | `core/tools/shell.rs` |

The one genuine gap: **OpenCode had no write path for reporting what
its work taught us.** Task transitions (`validate`/`complete`/`fail`)
conflate *state change* with *evidence*; they require leases and live
status (no post-completion user confirmation), carry no authority
distinction, no classification beyond the terminal status, and no
idempotency for redelivered reports. `remember`/`record_memory` are not
task-bound and do not feed learning as outcome evidence.

## 3. What P9 adds

Three additive pieces, no P0–P8 redesign:

### 3.1 `task outcome`: the ingestion seam

`ContextStore::record_task_outcome` (`context-runtime/tasks.rs`) +
one `task`-tool action (`mcp-server/mcp/mod.rs`). Input
(`TaskOutcomeInput`):

- `classification` (required, closed): `success | partial | failure |
  rejected | superseded` (`OutcomeClassification`, new).
- `summary` (required, ≤2048 chars, redacted): what happened.
- `evidence?` (≤2048, redacted): bounded detail (failing-test names,
  output digest — never logs/transcripts).
- `command?` (≤512, redacted): test/build command identity (never
  output). `exit_code?`: its status.
- `changed_areas?` (≤32 × ≤256, redacted): file references.
- `user_confirmed` (default false): explicit user speech act. `false`
  ⇒ authority `observed` (OpenCode-reported); `true` ⇒ authority
  `user_confirmed`. Never `ai_inferred` (that is `learn`'s job alone).
- `dedup_key?` (≤256): explicit idempotency, namespaced per task
  (`task_outcome:<task>:<key>`) so two tasks never collide.

Semantics (all store-enforced):

- The task must exist **in the caller's workspace** (same isolation
  error as every other task mutation — invisible, never leaked).
- **Any status accepted, including terminal**: confirmation routinely
  arrives after completion.
- **No lease, no row mutation, no transition**: appending evidence never
  changes who owns the task or what state it is in. Transition vs
  evidence vs confirmation vs lesson stay structurally separate (§7).
- **History outcome label is polarity-safe**: `success→success`,
  `partial→partial` (neutral), `failure→failure`,
  `rejected→rejected` (failure), `superseded→replaced` (neutral) — an
  abandoned approach can never read as a success pattern under the
  existing `outcome_polarity` mapping (§8). The verbatim
  classification always survives in summary + payload.
- **Bounded + redacted + deterministic**: every field redacted before
  storage (same authority as all task seams); the composed summary and
  payload pass through the history seam's truncate-with-marker caps;
  identical inputs (modulo `now`) produce identical rows.
- **Idempotent**: same (task, key) returns the original event with
  `duplicate=true` (first-write-wins; redelivery cannot rewrite
  history). Without a key every delivery is a new event (documented).
- **Atomic across processes**: the write runs in `BEGIN IMMEDIATE`
  (not deferred like the P5 transition paths), so concurrent
  cross-process writers serialize on the busy timeout — including the
  dedup check-then-act — instead of snapshotting stale reads and
  failing the upgrade (§13).

### 3.2 `HistoryKind::TaskOutcome`

One additive taxonomy variant (`task_outcome`, importance 80) mapped
into the existing P3 `Validation` kind-group. No learning-formula
change: success/failure outcomes weigh as strong evidence (1.0),
partial/superseded as medium (0.6); contradictions voice both sides
through the unchanged machinery. Schema untouched (kind is a string).

### 3.3 Open-path hardening (defect fixes found live, §16)

Concurrent multi-process outcome ingestion exposed two real defects in
the shared SQLite open path; both fixed in `context-runtime/db.rs`
with no behavior change on any success path (details §16).

## 4. What P9 explicitly does NOT do

- No new MCP tool (stays 25). No schema change (stays v7, no new
  tables). No new store, no second learning engine, no second history,
  no second ranking, no new trust system, no new retrieval.
- CodeBro never decides correctness, never runs tests for OpenCode,
  never executes commands on the outcome path, never inspects the repo
  autonomously, never calls a model, never invents conclusions, never
  promotes AI claims to facts, never schedules or spawns workers.
- A single outcome never becomes a best practice; a failure never
  auto-taints an approach; a user confirmation never auto-generalizes.
  Generalization requires P3 acceptance (≥3 support, contradiction
  checks); `USER_CONFIRMED` knowledge additionally requires the
  explicit `learn confirm` speech act.
- `based_on_version` is accepted but unused by `outcome` (no row
  mutates): documented, matching the shared-`TaskArgs` convention
  where irrelevant fields are ignored.

## 5. MCP surface

`TaskArgs` gains four optional fields (`classification`,
`user_confirmed`, `dedup_key`, `exit_code`) and reuses `summary`
(required for `outcome`), `reason` (evidence detail), `what` (command
identity), `changed_areas`. The `task` description and unknown-action
help name `outcome`. Responses are `{"action":"outcome","task_id",
"event_id","duplicate","classification","authority"}` through the
standard bounded envelope. `docs/MCP_API_V1.md` documents the action;
the P8 `integration::contract` needs no change (`outcome` rides the
existing `task_state` intent).

## 6. OpenCode contract (minimal)

```text
orient (workspace_context/context)
  → brief (engineering_brief)
  → [targeted follow-up: facts/impact/recall/memory/health]
  → OpenCode reasons, codes, executes WITH ITS OWN TOOLS
  → OpenCode reports outcome evidence (task outcome [+ validate/
    validation_result/complete/fail as the lifecycle requires])
  → restart-safe durable state
  → future briefs/recall surface validated outcomes
```

The probe E2E (`tests/p9_outcome_e2e.rs` probe 1) plays this exact
flow against the real binary without invoking any sandbox tool —
proof that CodeBro executes nothing on the outcome path.

## 7. Four concepts kept separate

`complete`/`fail` move task **status** (with the unchanged validation
gate). `outcome` appends task-bound **evidence** (this increment).
`user_confirmed=true` marks the report's **authority** (caller speech
act, like `remember`). `learn` acceptance produces **lessons**
(`AI_INFERRED`, evidence-bound, decaying). Collapsing any two would
recreate the conflation P9 was built to end; the E2E asserts status
and version are untouched by `outcome`.

## 8. Trust / provenance

Existing six-authority vocabulary, no parallel system. Mapping:

- OpenCode-reported test/build/result text ⇒ `observed` (default).
  Saying "tests passed" never becomes `USER_CONFIRMED`.
- User says "yes, this fixed it" (explicit `user_confirmed=true`) ⇒
  `user_confirmed` on that evidence event. It does NOT create a
  `USER_CONFIRMED` context record and does NOT generalize — that road
  still goes through `learn confirm` with its own speech act.
- Repeated corroborated outcomes ⇒ P3 `AI_INFERRED` (never
  self-confirmed; confidence-bounded; decaying; expiring).
- `superseded` outcomes ⇒ neutral polarity (`replaced`): preserved as
  history, invisible to success-pattern inference (pinned by
  `p9_superseded_outcomes_never_read_as_success`).
- `rejected` outcomes ⇒ failure polarity: preserved negative
  knowledge, can contradict success claims (pinned).

## 9. Learning integration

No learning-logic change beyond the one kind-group line (§3.2).
Flow: outcome events → existing detection (token-pair clustering,
deterministic ids) → existing evaluation (support/contradiction/
confidence ≥ 0.55, global bar) → `AI_INFERRED` record or
defer/reject. Verified live over the real binary: 1 outcome ⇒ 0
candidates; 3 shared-vocabulary failures ⇒ `failure_pattern`
accepted at 0.95 with `ai_inferred` explanation; the co-occurring
neutral cluster accepts only as `project_pattern`.

## 10. History / task / context / retrieval integration

- History: one bounded, redacted, session-linked, FTS-indexed event
  per report (plus nothing on redelivery). No transcripts, no payloads
  beyond 8 KiB, no row-id leakage beyond the citable event id.
- Task: `inspect` resume snapshots include outcome evidence in
  `recent_events` (≤10, oldest-first display) with zero snapshot
  changes.
- Context: single user-confirmed outcomes do NOT auto-create records;
  accepted inferences surface through the existing record pipeline.
- Brief: **unchanged**. Current-task outcomes arrive via `task_state`
  recent events; past outcomes via `history` excerpts (recall-ranked,
  ≤8); repeated outcomes via `learning` / `negative_knowledge`.
  Nothing new is surfaced noisily: relevance, scope, and bounds are
  the existing machinery's job. Proven by probe 1 (related brief
  surfaces the earlier failure; repeats byte-agree).

## 11. Bounds

Summary 2048 · evidence 2048 · command 512 · changed areas 32×256 ·
dedup key 256 · composed summary ≤2000+marker · payload ≤8192+marker
· response via the 256 KiB envelope. Oversize caller fields are
refused (caller restates); composed overflow truncates with an honest
marker (pinned by `outcome_maximal_inputs_stay_bounded_with_honest_markers`).

## 12. Security

Redaction at the write seam covers summary/evidence/command/areas
(store tests), the MCP response, `inspect`, `recall`, briefs, and
stderr (real-binary probes with two secret shapes on both authority
paths). Cross-workspace outcome injection refused with no content
leak (unit + MCP + real-binary). Malformed input (missing/unknown
classification, blank summary, ghost task, blank key) rejected with
bounded `-32602`. Unauthorized roots never reach the handler (P8
closure intact — `outcome` resolves through the same registry).
`task_outcome` is a new kind so no legacy rows exist; projections
carry only redacted-at-write content.

## 13. Determinism & concurrency

- Deterministic: identical inputs ⇒ identical summary/payload;
  dedup redelivery ⇒ identical id; brief repeats byte-agree
  (post-restart included); keyword-order/concurrency suites unaffected.
- In-process: the workspace mutation lock serializes `outcome` with
  every other task mutation (unchanged discipline).
- Cross-process: `BEGIN IMMEDIATE` serializes outcome writers on the
  busy timeout; same-key races resolve first-write-wins with both
  callers agreeing on the id (pinned live by probe 3, including a
  2-process same-key race).
- Restart: SQLite-durable; verified across hard kill + fresh process
  (probe 1).

## 14. Idempotency

Explicit `dedup_key`, per-task namespaced, backed by the P2
dedup-key mechanism inside the immediate transaction (atomic
check-then-act across processes). Same (task, key) ⇒ original id +
`duplicate=true`, content untouched (first-write-wins pinned).
Same key across tasks ⇒ distinct events. No key ⇒ new event per
delivery (documented, not a defect).

## 15. Schema

None. `OutcomeClassification`/`TaskOutcomeInput`/`TaskOutcomeRecord`
live in JSON-shaped history rows; `learning_candidates`,
`context_records`, `tasks`, and `task_checkpoints` are untouched;
`user_version` stays v7. Fresh, migrated (v1→v7 exercised by the
unchanged suites), restarted, and legacy-row behavior all preserved.

## 16. Hardening found live during P9 (root-caused, fixed, pinned)

Four-way concurrent outcome writers against the real binary produced
two failure classes, both root-caused to the shared open path (not to
P9 logic) and both fixed in `context-runtime/db.rs`:

1. **Spurious quarantine of a healthy database (CRITICAL, fixed).**
   FTS5 index validation reports lock contention as integrity-check
   *output rows* (`unable to validate the inverted index for FTS5
   table …: database is locked`), not as a query error — the busy
   timeout never covered it. `verify_integrity` read any non-`ok`
   output as corruption, quarantined the live database, and siblings
   then failed on the half-rebuilt replacement (cascade: further
   `NotADatabase`/check failures, phantom `does not exist` reads).
   Reproduced live (debris + warn line captured). Fix: contention
   lines report a synthesized busy error (retryable, never
   quarantinable); `DbError::Corrupt` now carries the offending check
   output for forensics.
2. **Unfenced quarantine (HIGH, fixed).** Any single failed check could
   destroy the database. Fix: bounded retries (6×100 ms, covering
   quarantinable *and* busy verdicts) plus a file-quiescence gate
   (withhold destruction while the file churns; a vanished file means
   a sibling already quarantined) plus pid-unique debris names (no
   more same-second collisions).
3. **Deferred-txn write races (MEDIUM, fixed).** Concurrent
   cross-process check-then-act failed fast with `database is locked`
   (snapshot-upgrade races the busy timeout does not retry). Fix
   (P9-scoped, no P5 path touched): `record_task_outcome` uses
   `BEGIN IMMEDIATE` with explicit rollback discipline.
4. **Silent evidence-boundary bypass (LOW, fixed during testing).**
   The first implementation composed the history summary past the
   2000-char budget (2048 summary + 512 command) outside the history
   seam's truncation. Fix: the composed line passes through
   `clean_text` exactly like every P5 transition event.

After the fixes: 6/6 clean 4-process runs, rabid-open unit stress
(8 threads × hostile open/write/close) leaves zero debris with
content intact, and the genuine-corruption path still quarantines
(pinned).

## 17. Testing

| Suite | New tests | What they pin |
|---|---|---|
| `tasks::tests` (store) | 10 | classification roundtrip + polarity-safe labels; evidence without transition (status/version untouched, snapshot surfacing); terminal-task + user-confirmed authority; per-task dedup + key isolation + keyless behavior; unknown/cross-workspace refusal; bounds refusal; secret redaction; deterministic structured evidence; first-write-wins replay; maximal-input budgets |
| `learning_tests` | 3 | failed outcomes ⇒ `failure_pattern`; superseded ⇒ neutral `project_pattern` (never success/failure); success/failure contradiction voicing |
| `recall` | 2 | keyword surfacing of outcome evidence (the feedback loop); cross-workspace invisibility |
| `db` | 3 | 8-thread shared-connection storm (no debris, all ops succeed); rabid open/close storm (no debris, content survives); genuine corruption still quarantines; contention-is-busy classification |
| `mcp::task_tests` | 6 | evidence without transition over MCP; post-completion authority split; malformed matrix + help text; MCP dedup; MCP isolation; MCP redaction (both authorities) |
| `tests/p9_outcome_e2e.rs` (real binary) | 3 | full loop (ingest → no-transition → dedup → validate/complete → user confirmation → recall → restart → related-brief feedback → determinism); adversarial (2 secret shapes, malformed matrix, cross-workspace refusal, stderr hunt); 4-client concurrency + same-key race |
| Live adversarial probes (throwaway, §18) | — | single outcome ⇒ 0 candidates; 3 failures ⇒ accepted `failure_pattern` (0.95, `ai_inferred`) with secret-bearing summary redacted end to end |

Full workspace suite: **1454 passed / 0 failed** (1426 baseline + 28).
`cargo clippy --workspace --all-targets -- -D warnings` clean.
`cargo fmt --check` clean. `scripts/check_workspace_deps.sh` OK
(no manifest change — zero new dependencies).

## 18. Real-binary E2E & real-OpenCode E2E

- **Real-binary E2E: PASS** (`tests/p9_outcome_e2e.rs`, 3 probes,
  hermetic tempdirs + `CODEBRO_STATE_DIR`/`CODEBRO_SKILLS_DIR`, strict
  stdout-purity validator on every call, `~/.codebro` untouched).
- **Live adversarial probes: PASS** (throwaway harnesses, §17):
  CodeBro executed nothing (no sandbox tool invoked in any flow).
- **Real-OpenCode E2E: PASS** — OpenCode 1.18.29
  (`agnes/agnes-2.5-flash`), hermetic HOME/config/repo/state/skills,
  `CODEBRO_STATE_DIR` isolation. Session 1: orient
  (`workspace_context`) → brief (`unknown` freshness + honest
  `NO_RELEVANT_TESTS`) → `task` create/start (`task::f0e7…`) → the
  agent added `negate` + a `negates` test with its own tools and ran
  `cargo test` itself (2 passed) → `task outcome` success
  (`dedup_key: negate-1`, event 3, `observed`) → `validate` →
  `validation_result passed` → `complete` → agent-reported
  confirmation that CodeBro ran zero tests and made zero edits.
  Session 2 (fresh process, same state): `task list` shows the
  completed task; a related brief
  ("absolute-value helper near negate and double") surfaces the
  earlier outcome verbatim in its history excerpts; read-only (no
  edits, no tests). State independently verified (SQLite rows, git
  diff confined to the agent's `src/lib.rs` edit, no quarantine
  debris, hermetic dirs only).

## 19. Documentation

- This document (required). `CHANGELOG.md` (P9 entry). `AGENTS.md`
  (P9 contract section + `task outcome` in the tool table).
  `docs/MCP_API_V1.md` (`task` row documents `outcome`; contract
  preamble notes P9 reporting). `P9_POST_IMPLEMENTATION_AUDIT.md`
  (dedicated adversarial audit).

## 20. Known debt (honest, non-blocking)

1. Real-OpenCode legs carried no secret-shaped inputs (clean text by
   design); redaction on the agent-driven path is covered by the
   hermetic real-binary probes instead (§12, §18).
2. Cross-process writers still share the documented single-writer
   *performance* assumption: bursts beyond the retry bound surface
   busy errors for the caller to retry (degrade, never destroy).
3. `based_on_version` is ignored by `outcome` (documented §4).
4. `superseded` history outcome label is `replaced` (polarity-safe
   alias; classification verbatim in payload/summary).
5. Partial outcomes are terminal-agnostic evidence; there is still no
   `partial` task *status* (by design — statuses were not duplicated).
6. All P0–P8 carried debts remain as documented.

## 21. Final status

| Criterion | Result |
|---|---|
| Outcome model coherent, no duplicated engines | PASS |
| OpenCode/CodeBro boundary clean (CodeBro executes nothing) | PASS (no sandbox call in any P9 flow) |
| No secret leakage (write + all projections + stderr) | PASS |
| Cross-workspace isolation | PASS |
| P8 authorization intact | PASS (same registry path) |
| Malformed input rejected | PASS |
| Provenance/authority gates preserved | PASS |
| Rejected/superseded cannot become positive knowledge | PASS |
| Restart / migration / idempotency | PASS |
| Determinism (repeat, reorder, concurrency, restart) | PASS |
| Real-binary MCP E2E | PASS |
| Real-OpenCode E2E | PASS (§18: two live legs, agent-owned edits/tests) |
| P0–P8 regression (1454/1454), clippy, fmt | PASS |
| No `~/.codebro` pollution, no real repo/skill mutation | PASS (hermetic dirs; agent edits confined to the hermetic probe repo) |
| Adversarial post-implementation audit | PASS with 4 fixed findings (`P9_POST_IMPLEMENTATION_AUDIT.md`; no residual critical/high/medium) |

**P9 COMPLETE WITH NON-BLOCKING DEBT** (§20).
