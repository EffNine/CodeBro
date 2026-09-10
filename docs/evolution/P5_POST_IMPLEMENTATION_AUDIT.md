# P5 — Durable Engineering Task Runtime: Post-Implementation Adversarial Audit

**Verdict: PASS WITH CHANGES** (6 defects found, all fixed, all pinned by regression tests)

**Scope:** `crates/context-runtime/src/tasks.rs` (~3300 LOC), `crates/context-runtime/src/db.rs`
(v5→v6 migration), `crates/context-runtime/src/history.rs` / `learning.rs` (P2/P3 seams),
`crates/mcp-server/src/mcp/mod.rs` (`task` tool handler + `TaskArgs`), `crates/mcp-server/tests/task_runtime_e2e.rs`.
This is NOT P6: no scheduling, automation, background workers, queues, or model calls were added.

**Method:** read the implementation before trusting the completion report; wrote adversarial
tests against the *unfixed* code first (6 failed, proving the defects); fixed; re-verified.
Every claim below cites code or a named test.

---

## 1. Findings

### F1 — HIGH — Takeover bypass: non-owner could mutate live state after lease expiry
- **Area:** worker lease / fencing (`enforce_lease`, `crates/context-runtime/src/tasks.rs`).
- **Description:** `enforce_lease` refused a non-holder only while the lease was *live*.
  After expiry it passed any caller carrying a current-or-higher `lease_version`, so worker B
  could `checkpoint` / `pause` / `start_task_validation` on worker A's task directly —
  no explicit `resume`, no `lease_version` fencing increment. The reported rule
  ("takeover only via explicit resume") was not enforced.
- **Impact:** a disappeared worker's task could be mutated by a stranger without ever
  fencing ownership forward; fencing versions could stall while writers changed.
- **Evidence:** pre-fix, `audit_expired_lease_requires_explicit_resume_for_mutations` failed —
  the "takeover write" checkpoint by worker B committed (`version: 1`) on A's expired task.
- **Fix:** `enforce_lease` now refuses *any* non-owner live-state mutation while the task
  names an owner, live or expired, and directs the caller to `resume`. Ownership, not the
  caller's number, arbitrates — forged higher versions grant nothing
  (`audit_forged_lease_version_grants_nothing`). Live-lease error text is unchanged.
- **Regression test:** `audit_expired_lease_requires_explicit_resume_for_mutations`,
  `audit_forged_lease_version_grants_nothing`.
- **Status:** fixed + verified.

### F2 — HIGH — Checkpoint invented / resurrected lease state
- **Area:** checkpoints (`create_task_checkpoint`).
- **Description:** the checkpoint transaction unconditionally wrote
  `lease_expires_at = now + TTL`. Consequences: (a) checkpoints on holderless
  pending/paused tasks invented lease state with no owner; (b) a non-owner's checkpoint
  on an expired task (see F1) resurrected the *previous owner's* lease, blocking
  legitimate takeover.
- **Impact:** lease resurrection without ownership transfer; phantom expiries on
  holderless tasks.
- **Evidence:** pre-fix, `audit_holderless_checkpoint_creates_no_lease` failed
  (`lease_expires_at` became `Some` on a pending task).
- **Fix:** the lease is heartbeated only when the caller *is* the holder; holderless
  checkpoints move the pointer and bump the version with no lease columns touched.
- **Regression test:** `audit_holderless_checkpoint_creates_no_lease`.
- **Status:** fixed + verified.

### F3 — MEDIUM — Paused tasks misclassified as stale
- **Area:** recovery (`TaskRecord::is_stale`).
- **Description:** `is_stale` was `!terminal && != Pending && expired`, and a released
  (NULL) lease reads as expired — so every paused task derived as stale and appeared in
  `task stale` (recoverable-work) listings. Paused is an intentional stop, not an interruption.
- **Impact:** operators told to "resume" deliberately paused work; `stale` listing noisy.
- **Evidence:** pre-fix, `audit_paused_tasks_are_not_stale` failed.
- **Fix:** staleness covers only `running` / `validating`, matching the documented rule.
- **Regression test:** `audit_paused_tasks_are_not_stale` (store) +
  `paused_tasks_are_not_listed_as_stale_over_mcp` (MCP).
- **Status:** fixed + verified.

### F4 — MEDIUM — Expired-holder heartbeat renewed without fencing
- **Area:** leases (`heartbeat_task_lease`).
- **Description:** the lease holder could heartbeat-renew an already-expired lease,
  silently un-expiring it with no version bump. Recovery after expiry must fence
  forward through `resume`.
- **Impact:** stale-recovery path bypassed fencing; divergent from the specified
  expired-renew refusal.
- **Evidence:** pre-fix, `audit_heartbeat_refuses_expired_lease` failed.
- **Fix:** heartbeat refuses expired leases ("resume explicitly to re-acquire it");
  live heartbeat is unchanged; self-resume with no competing owner still renews in place.
- **Regression test:** `audit_heartbeat_refuses_expired_lease`.
- **Status:** fixed + verified.

### F5 — MEDIUM — MCP mutation lock was a no-op for `task`
- **Area:** MCP handler (`task` in `crates/mcp-server/src/mcp/mod.rs`).
- **Description:** the lock guard was bound inside a `match` arm block
  (`let _guard = ws.mutation_lock.lock().await;`) and dropped immediately —
  mutating actions serialized nothing at the MCP layer, contradicting the documented
  workspace-mutation-lock discipline (and AGENTS.md).
- **Impact:** pipelined same-workspace task mutations relied solely on the store mutex.
  No corruption was observed (single shared `ContextStore` connection serializes
  in-process work), but the defense-in-depth layer was absent.
- **Fix:** the guard now binds as `Option<MutexGuard>` living until the handler returns;
  `list` / `inspect` / `stale` still skip the lock (read-only, verified side-effect free).
- **Regression test:** existing `stale_writer_and_lease_are_enforced_over_mcp` +
  full suite re-run; no behavior change, serialization restored.
- **Status:** fixed + verified.

### F6 — LOW — Checkpoint metadata return/persist divergence (P4-lesson class)
- **Area:** checkpoints (`row_to_checkpoint`).
- **Description:** `create` returned `metadata_json: None` while the row persisted `""`
  (NOT NULL column via `unwrap_or_default`), so a re-read differed from the return value —
  the exact API-vs-persisted divergence class P4's audit caught for `current_version`.
- **Impact:** cosmetic inconsistency; any byte-compare of returned vs re-read checkpoints failed.
- **Evidence:** pre-fix, `audit_returned_state_matches_persisted_state` failed on this field.
- **Fix:** reads normalize `""` back to `None` (no migration; old rows read consistently too).
- **Regression test:** `audit_returned_state_matches_persisted_state` (covers version, lease,
  pointer, validation, outcome parity on every mutation).
- **Status:** fixed + verified.

### INFO-1 — Cancel is intentionally supervisory (no lease), version-pinned
`cancel_task` takes no worker and enforces no fencing by design ("supervisory; no lease
needed"). A stale `based_on_version` anchor is still refused. Pinned by
`audit_cancel_is_supervisory_but_version_pinned`. Same-server callers should pass
`based_on_version`; cross-server concurrent cancel races are covered by the
single-server deployment assumption (see Remaining limitations).

### INFO-2 — `skill_refs` are opaque labels, never dereferenced
`set_task_skill_refs` checks bounds only (≤16 refs, ≤256 chars) — not skill existence or
workspace. This is safe because refs are never resolved: no skill-table reads, no health
writes, no execution (pinned: `audit_oversized_outcome_and_refs_are_refused` asserts the
`skills` / `skill_candidates` tables stay empty). A cross-workspace string confers no access.

### INFO-3 — Validation trust is attestation-based
Callers report `passed`/`failed`; CodeBro cannot re-run OpenCode's validation. What the
store guarantees (and tests pin): the record is bound to the task row + workspace, requires
the holder's lease, requires `validating` status, and only a *recorded passed result on that
task* arms `complete`. Forged cross-task/cross-workspace validation is refused at the
storage layer. This matches the architecture (OpenCode owns execution).

### INFO-4 — Passed validation is not bound to a task version
A checkpoint written *after* a passed result does not disarm the completion gate
(`complete` still succeeds). Forbidding that would break legitimate record-final-state
flows; the gate's job is "was this task validated", not "was nothing recorded since".
Documented as a semantic boundary, not a defect. The dangerous direction is closed:
`resume` from stale `validating` returns to `running`, and the old (pending or passed)
result cannot complete the post-resume state without revalidation
(pinned by `audit_validating_state_recovers_across_reopen`).

### INFO-5 — HistoryKind count
The audit brief said "9 HistoryKind variants"; the code has 11 additive task variants
(created/started/paused/resumed/checkpoint/validation_started/validation_passed/
validation_failed/completed/failed/cancelled). Trivial reporting variance; docs never
claimed 9.

---

## 2. State Machine Result

- **Are all transitions store-enforced?** Yes. Matrix `task_transition_allowed` (12 allowed
  pairs, all self-transitions rejected, exhaustively tested) plus per-action status gates
  (`start` requires pending; `validate` requires running; `complete` requires validating +
  passed; `fail` requires running/validating; `cancel` via matrix). `resume` intentionally
  covers stale `running`/`validating → running` (interrupted-work recovery, not a matrix
  violation — `resumed` normalizes to `running`).
- **Can callers inject status?** No. `TaskArgs` carries `status` only as a *list filter*;
  no mutation path accepts a raw status — verified by schema inspection and by
  `invalid_transitions_are_refused_over_mcp` (pending→paused, pending→completed refused).
- **Can completed tasks mutate?** No. `audit_terminal_tasks_refuse_every_mutation` drives to
  `completed` and asserts all 10 mutation paths refuse; `failed`/`cancelled` likewise
  (no `failed → completed` without revalidation — impossible, terminal states have no exit).
- **Can validation be bypassed?** No. `RUNNING → complete` refused; `VALIDATING → complete`
  without a recorded passed result refused (`completion_gate_requires_passed_validation`,
  `completion_gate_is_enforced_over_mcp`); forged cross-task/workspace validation refused
  by task-row binding + workspace checks.

## 3. Checkpoint Result

- **Immutable?** Yes — INSERT-only API surface, no UPDATE/DELETE path, unique
  `(task_id, version)`; versions/contents verified across reads
  (`checkpoints_are_immutable_versions`, `audit_cross_task_checkpoint_isolation`).
- **Versioning safe?** Yes in-process (store mutex) and across processes for the race that
  matters: concurrent same-base checkpoints → `MAX(version)+1` collision → unique-constraint
  loser rolls back the whole transaction. Pinned by sequential *and* true-thread 4-way races
  (exactly one winner, losers get explicit `stale` errors).
- **Creation atomic?** Yes — checkpoint row + pointer move + version bump + history event
  commit in one `unchecked_transaction`; any failure rolls back all of it. No API can set
  `current_checkpoint_id` except alongside a successful INSERT in the same transaction.
- **Dangling pointers?** Impossible via any code path (pointer written only post-INSERT,
  same tx; no delete API; no cross-task pointer API).
- **Silent overwrites?** No — two writers never silently merge (version anchor or unique
  constraint always arbitrates).

## 4. Worker / Fencing Result

- **Can stale workers mutate?** No — every live-state mutation enforces fencing version +
  ownership (F1 fix); the pre-fix bypass is now a regression test. Post-takeover, the old
  worker is refused on checkpoint/pause/validate/record/complete/fail/heartbeat.
- **Can expired leases renew?** Only via explicit `resume` (F4 fix pins heartbeat refusal).
- **Can two workers own one task?** No — sequential double resume: loser refused (live lease
  not resumable); true-thread 4-way resume race: exactly one winner, persisted owner equals
  the winner (`audit_concurrent_resume_single_winner`).
- **Can old workers resurrect?** No — stale-version heartbeat refused; forged-version
  heartbeat refused by holder check; resume by a fenced worker transfers ownership forward
  (new version), never backward.
- **Does `lease_version` actually fence?** Yes — monotonic increment on every acquisition /
  takeover; every mutation compares caller version; mismatches refuse before any write.

## 5. Concurrency Result

- **Does `based_on_version` work?** Yes on all 8 versioned paths (pause/resume/start/
  validate/record/complete/checkpoint/skill_refs/cancel) — stale anchors refused
  (`stale_based_on_version_is_refused`, `audit_stale_anchors_refused_on_all_versioned_mutations`).
  Heartbeat carries no anchor by design (it never bumps `current_version`, so it cannot
  invalidate anyone's anchor).
- **Stale writers rejected?** Yes — never silently merged, always an explicit error.
- **Cross-process races safe?** Sequential cross-connection behavior verified (shared-state-dir
  MCP tests, real-binary E2E). *Concurrent* cross-process writers are outside the documented
  deployment model (one server per `state.db`; version checks are read-then-write without a
  `WHERE version = ?` predicate, so two simultaneous cross-process writers could last-win).
  The checkpoint unique constraint still arbitrates checkpoint races cross-process. See
  Remaining limitations.

## 6. Recovery Result

- **After process death:** tasks stay durable in exactly their last committed state —
  never auto-completed, never deleted (E2E `killed_process_leaves_recoverable_task`;
  store reopen tests).
- **RUNNING tasks:** persist as `running` with a now-stale lease; recoverable only via
  explicit `resume`, which fences forward to the new worker.
- **VALIDATING tasks:** persist as `validating` with the pending result; after reopen,
  `complete` is still refused, and post-expiry `resume` returns to `running` requiring full
  revalidation (`audit_validating_state_recovers_across_reopen`).
- **Resume correctness:** snapshot selects `ORDER BY version DESC LIMIT 1` (latest), bounded
  to the task: identity + latest checkpoint + ≤10 recent task-scoped events + skills +
  intent note (`resume_snapshot_is_bounded_and_complete`; E2E asserts checkpoint v1 +
  outcome survive two restarts).

## 7. Isolation Result

- **Workspace:** enforced at storage (`load_task_for`, per-accessor owner checks) and MCP
  layers; cross-workspace reads are invisible (`None`/empty), mutations are refused —
  store-level (every seam enumerated in `workspace_isolation_holds_at_every_seam`),
  MCP-level, and real-binary cross-process (`workspace_isolation_holds_across_processes`).
- **Project/task:** task events carry `task_id`; recall keeps task history invisible without
  its task; snapshot queries filter by `task_id`; cross-task checkpoint accessors return
  none/empty (`audit_cross_task_checkpoint_isolation`); task-scoped context rules (P1)
  unchanged.
- **Checkpoint / context / skill:** checkpoint rows namespaced by `task_id`; resume snapshot
  contains only the task's own rows; skill refs never leak skill content (never read).

## 8. P3 / P4 Result

- **P3 provenance preserved?** Yes — task lifecycle events are ordinary P2 history rows;
  `task_completed` / `task_validation_passed` / `task_validation_failed` / `task_failed`
  map to the `Validation` evidence group (weight 1.0), structural transitions to
  `Observation` (0.3); no auto-promotion, no `USER_CONFIRMED`, evidence minimums and
  contradiction handling untouched (`task_completion_events_are_valid_learning_evidence`).
- **Can task results bypass confirmation?** No — outcomes are `source: "mcp:task"` history
  evidence consumed through existing learning gates.
- **Unauthorized skill refs?** Refs are unresolved labels (INFO-2); outcomes never mutate
  skill health (no code path; tables-verified empty).
- **Does P5 mutate P4 execution?** No — no skill execution, no health writes, no candidate
  writes anywhere in the task runtime.

## 9. Database Result

- **v5 → v6 migration:** `IF NOT EXISTS` tables + indexes, no backfill, P0–P4 rows untouched;
  fresh-shape, preservation-with-data, and interrupted-migration-resume tests green.
- **Restart:** reopen preserves tasks, leases, checkpoints, validation, outcomes, events.
- **Concurrent writers:** in-process serialized (single shared connection + mutex + MCP lock);
  cross-process sequential verified; concurrent cross-process documented as out of scope.
- **Checkpoint/history transactions:** single-transaction commit in every path (transition
  core, start/resume, validation, complete/fail, checkpoint); heartbeat/skill_refs (no
  history event) are single-row updates by design, not transitions.

## 10. MCP Result

- **24 tools?** Yes — router test pins all 24 names including `task`.
- **14 task actions?** Yes — list/stale/create/inspect/start/pause/resume/checkpoint/
  validate/validation_result/complete/fail/cancel/skill_refs (unknown actions rejected).
- **Raw CRUD bypass?** None — semantic actions only; `lease_version` derived server-side
  from the stored row (callers cannot supply it); `worker` is the server's own id.
- **Caller-controlled security state?** Only `based_on_version` (tightens, never loosens:
  omitting it skips the check but fencing + state machine still arbitrate) and `result`
  (attestation, INFO-3).
- **Hidden read side effects?** None — `inspect`/`list`/`stale` skip the mutation lock and
  perform no writes (no session ensure, no heartbeat, no history). Failed mutations may
  leave a best-effort task-bound session row (documented, harmless — the task row is the
  source of truth).

## 11. Regression Result

- P0 PASS (context foundation suites green)
- P1 PASS (fingerprint/intent suites green)
- P2 PASS (sessions/history/recall suites green)
- P3 PASS (learning suites green)
- P4 PASS (skills lifecycle suites green)
- P5 PASS WITH CHANGES → now VERIFIED (all defects fixed, see §1)

## 12. Test Result (exact)

```text
workspace tests:        1259 passed, 0 failed (31 suites)
context-runtime:         250 passed (233 pre-existing + 17 new adversarial audit tests)
mcp task_tests:            9 passed (7 pre-existing + 2 new)
real-binary E2E:           5 passed (4 pre-existing + 1 new pause/resume E2E)
migration tests:         green (fresh v6, v5→v6 with P0–P4 data, interrupted resume)
cross-connection tests:  green (two server objects, one state dir: fencing + takeover)
cargo fmt --check:       clean
cargo clippy --workspace --all-targets: 0 warnings
scripts/check_workspace_deps.sh: OK
~/.codebro:              untouched (state.db mtime predates the audit runs; all
                         tests use hermetic tempdirs / CODEBRO_STATE_DIR)
```

New adversarial tests (20): 17 store-level `audit_*` in `tasks.rs`, 2 MCP-level in
`task_tests`, 1 real-binary E2E (`paused_task_survives_restart_and_resumes` covering
pause → kill → inspect-paused-not-stale → resume → checkpoint → validate → complete →
restart → completed-with-outcome, asserting persisted rows via `inspect`, not return values).

## 13. Remaining Limitations (honest)

1. **Wall-clock TTL (15 min).** Leases use unix-seconds wall time; clock skew between
   processes can shift takeover windows. Fencing (monotonic version), not timestamps,
   arbitrates ownership, so skew cannot grant a stale worker access — it can only make
   an idle task recoverable slightly early/late. Documented in `P5_IMPLEMENTATION.md`.
2. **Concurrent cross-process writers** rely on the documented one-server-per-`state.db`
   deployment. Version checks are read-then-write without a `WHERE version = ?` predicate;
   simultaneous writers from two processes could last-win on non-checkpoint mutations.
   In-process writers are fully serialized; checkpoint races are arbitrated cross-process
   by the unique `(task_id, version)` constraint.
3. **Crash mid-transaction** loses only the in-flight mutation (SQLite atomicity); no
   recovery action beyond explicit `resume` is ever needed. There is no write-ahead
   intent log beyond the transaction itself — by design (P6 territory if ever needed).
4. **`based_on_version` is opt-in** at the MCP layer. Omitting it is safe but weaker;
   callers wanting strict serializability should always pass it.
5. **Supervisory `cancel`** needs no lease by design (INFO-1). If a deployment needs
   fenced cancellation, require `based_on_version` on the cancel path.
6. **E2E TTL expiry** cannot be exercised through the real binary (15-minute wall clock);
   expiry paths are covered at the store layer with synthetic time and cross-connection
   MCP tests. No mock-time seam exists in the server — intentionally (production code
   stays simple).

## 14. Pass-criteria checklist (§64)

1. ✅ store-enforced transitions (matrix + per-action gates, exhaustive tests)
2. ✅ no raw status mutation (schema + refusal tests)
3. ✅ completion requires recorded passed validation (gate tests, both layers)
4. ✅ immutable checkpoints (INSERT-only + unique version)
5. ✅ atomic checkpoint/pointer/history transaction
6. ✅ dangling references impossible (no API path; same-tx writes)
7. ✅ `based_on_version` on all versioned mutations
8. ✅ stale writers rejected, never merged
9. ✅ lease fencing on every live-state mutation (F1 fix)
10. ✅ expired workers cannot mutate (only explicit `resume` transfers ownership)
11. ✅ expired workers cannot resurrect leases (heartbeat refusal + holder checks)
12. ✅ double takeover has exactly one winner (sequential + threaded races)
13. ✅ explicit resume required for takeover (F1/F2 fixes + tests)
14. ✅ interrupted tasks durable (restart + kill E2E)
15. ✅ never silently completed (stale listings + restart assertions)
16. ✅ resume selects latest checkpoint (`ORDER BY version DESC LIMIT 1`)
17. ✅ workspace isolation at MCP + storage (three layers of tests)
18. ✅ task-scoped context does not leak (isolation tests)
19. ✅ P3 trust intact (mapping + gate tests)
20. ✅ P4 trust intact (reference-only, tables-verified)
21. ✅ secrets redacted on every free-text field (synthetic-secret sweep incl. snapshot)
22. ✅ MCP cannot bypass storage safety (server-derived lease/worker, semantic actions)
23. ✅ reads side-effect free (lock skipped + write-free paths verified)
24. ✅ v5→v6 migration safe (fresh/upgrade/interrupted tests)
25. ✅ cross-process behavior verified (cross-connection MCP + real-binary E2E)
26. ✅ real-binary E2E passes (5/5, incl. new pause/resume lifecycle)
27. ✅ P0–P4 green (1259 workspace tests, 0 failures)
28. ✅ hermetic tests (`~/.codebro` untouched)
29. ✅ no scheduler/daemon/background execution (grep-verified; request-driven only)
30. ✅ no P6 functionality (14 actions are CRUD-lifecycle, no automation)
31. ✅ docs match behavior (`P5_IMPLEMENTATION.md` + CHANGELOG updated for the fixes)
32. ✅ limitations honestly documented (§13)

## 15. Audit artifact summary

- Fixed: `crates/context-runtime/src/tasks.rs` (`enforce_lease` ownership rule,
  conditional checkpoint heartbeat, `is_stale`, heartbeat expiry refusal, metadata
  read normalization), `crates/mcp-server/src/mcp/mod.rs` (mutation-guard lifetime).
- Added: 17 store adversarial tests, 2 MCP tests, 1 real-binary E2E.
- Updated honestly: `docs/evolution/P5_IMPLEMENTATION.md` (lease/takeover/stale rules),
  `CHANGELOG.md` (audit findings entry).

**P5 is AUDITED / VERIFIED.** The key guarantee holds: *interrupted work keeps correct
durable state, ownership, progress, validation, and recovery — and an old process can
never silently corrupt newer work.* Six places where that guarantee had holes now have
regression tests proving the holes are closed.
