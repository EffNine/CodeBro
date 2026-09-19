# CodeBro P9 Post-Implementation Adversarial Audit

**Method:** active break attempts, not code review. Every finding below
was reproduced live against the real binary or the store API before it
was fixed; every fix was re-probed until the attack failed. Throwaway
harnesses in `/tmp/opencode` (repros) plus permanent regressions in the
suite. Scope: all new P9 surface (`task outcome`, `task_outcome`
history, learning consumption, open-path changes) and every P9 security
obligation from the mission (§SECURITY 1–18).

**Verdict:** 1 CRITICAL + 1 HIGH + 1 MEDIUM + 1 LOW found live and
fixed with regressions. No residual critical/high/medium. Two
non-blocking notes (§7).

---

## F1 (CRITICAL, FIXED): concurrent opens quarantined the healthy database

**Attack:** 4+ simultaneous `codebro serve` processes sharing one
`state.db`, each writing on startup (the P9 E2E concurrency shape).

**Reproduced:** worker opens failed; `state.db.corrupt-*` debris
appeared next to a live database; subsequent reads returned `task …
does not exist` for committed tasks; siblings then failed on the
half-rebuilt replacement (cascade: further `NotADatabase`/check
failures, phantom reads).

**Root cause:** `PRAGMA quick_check` validates FTS5 indexes, and FTS5
reports lock contention as check *output rows* (`unable to validate
the inverted index for FTS5 table …: database is locked`) — not as a
query error, so the busy timeout never covered it. `verify_integrity`
read any non-`ok` output as corruption → quarantine → destruction of a
healthy database. (Why unit/python probes missed it: no FTS5 tables in
the toy schemas.)

**Fix (`context-runtime/db.rs`):** contention lines report a
synthesized busy error (retryable, never quarantinable);
`DbError::Corrupt` carries the offending check output; bounded retries
(6×100 ms) cover quarantinable *and* busy verdicts; a file-quiescence
gate withholds destruction while the file churns (a vanished file
means a sibling already quarantined); debris names are pid-unique.

**Regressions:** `db::concurrent_open_tests` (shared-connection
storm, rabid open/close storm, genuine-corruption quarantine,
contention-is-busy classification); `tests/p9_outcome_e2e.rs` probe 3
(4 live processes + same-key race); 6/6 clean multi-process runs
post-fix (was ~50% quarantine pre-fix).

## F2 (HIGH, FIXED): quarantine cascade + debris collisions

**Attack:** consequence of F1 — two processes quarantining the same
instant collided on the seconds-resolution debris name, and racing
opens read half-built replacement files (`NotADatabase`, version-0
mid-migrate views, detached-inode writes).

**Fix:** quiescence gate (above) breaks the cascade at the source;
pid-suffixed debris preserves every quarantiner's evidence. Verified:
zero debris across all contention suites; genuine corruption still
quarantines exactly once with complete debris.

## F3 (MEDIUM, FIXED): cross-process write races on the outcome path

**Attack:** same-key and distinct-key concurrent outcome deliveries
from separate processes.

**Reproduced:** `database is locked` (deferred-txn snapshot-upgrade
races the busy timeout does not retry) and racy dedup
check-then-act.

**Fix (P9-scoped, no P5 path touched):** `record_task_outcome` runs in
`BEGIN IMMEDIATE` with explicit rollback discipline — writers
serialize on the busy timeout and the dedup probe is atomic across
processes. Verified: 4-process distinct-key runs all succeed with
distinct ids; 2-process same-key races agree on one id with exactly
one creator (probe 3).

## F4 (LOW, FIXED): composed summary bypassed the history budget

**Found in testing:** the composed outcome line (2048-char summary +
512-char command) exceeded the 2000-char history budget because the
new path called the raw in-transaction insert instead of the cleaning
wrapper every P5 transition uses.

**Fix:** the composed line passes through `clean_text` (payload
already went through `clean_payload`). Pinned by
`outcome_maximal_inputs_stay_bounded_with_honest_markers`.

## Security sweep (all held)

| # | Probe | Result |
|---|---|---|
| 1 | Secret as outcome summary | redacted at write; absent from inspect/recall/brief/stderr |
| 2 | Secret as evidence (`reason`) | same |
| 3 | Secret in command / changed areas | same (changed-areas unit + MCP secret tests) |
| 4 | Secret in completion note | pre-existing seam, unchanged, suite-green |
| 5 | Secret in user confirmation | redacted (E2E second secret shape, `user_confirmed=true` path) |
| 6 | Secret in learning candidate | 3-outcome live run: proposition/inference/context all clean |
| 7 | Secret in history | `record_history` seam redacts; FTS clean (P2 suite) |
| 8 | Secret in brief projection | E2E brief hunt clean; `[REDACTED]` marker present |
| 9 | Malformed MCP args (5-shape matrix + ghost task) | bounded `-32602`, no state touched |
| 10 | Cross-workspace outcome injection | refused, no leak (unit + MCP + real-binary) |
| 11 | Outcome on another workspace's task | refused (`another workspace`) |
| 12 | Unauthorized `workspace_root` | refused pre-state by the P8 registry (path unchanged) |
| 13 | Legacy rows with secrets | vacuous for `task_outcome` (new kind); all rows redacted at write |
| 14 | Restart persistence | task + checkpoint + outcomes survive kill + fresh process |
| 15 | Concurrent writes | F1–F3 above; post-fix green |
| 16 | Repeated identical submissions | dedup: same id + `duplicate=true`, first-write-wins (content frozen) |
| 17 | Superseded resurfacing | neutral polarity; never a success pattern (learning test + live) |
| 18 | Rejected as positive knowledge | failure polarity; contradicts success claims (live + unit) |

Trust gates also re-probed live: single outcome ⇒ zero learning
candidates (no auto-best-practice); 3 failures ⇒ one accepted
`failure_pattern` (0.95, `ai_inferred`, never `user_confirmed`); no
`USER_CONFIRMED` record is created by any outcome path (only `learn
confirm` with its own speech act can); `observed` is the default
authority and cannot be upgraded by relevance.

## Non-blocking notes

1. **Bursts beyond the retry bound surface busy errors.** Six
   attempts × 100 ms absorb realistic multi-client bursts (2 clients,
   model-paced calls); a sustained open/write hammer may still return
   busy from individual opens. Callers degrade/retry per the P8
   contract — availability, never destruction. Accepted as the
   documented cross-process performance posture.
2. **Real-OpenCode legs carried no secret-shaped inputs.** Redaction
   on the agent-driven path is covered by hermetic real-binary probes
   (two secret shapes, both authorities); the live agent runs used
   clean text by design.

## Residual risk statement

No known way remains to destroy, leak across workspaces, or falsely
promote knowledge through the P9 surface within the tested threat
model (same-user MCP client, multi-process concurrency, malicious or
accidental malformed/secret-bearing input). The quarantine path —
the only destructive operation in the store — now requires
corroborated failure on a quiescent file and leaves pid-unique
forensic debris.
