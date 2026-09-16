# Phase-1 Controlled ON/OFF Evaluation — PROPOSAL (DRAFT, NOT FROZEN)

**Status: DRAFT — NOT FROZEN. NO TRIALS UNTIL A FROZEN REVISION OF THIS SHEET IS SIGNED.**
**Date drafted:** 2026-09-16. **Repo HEAD at draft:** `329c1e255c`.
**Depends on:** Phase-0 gate PASS (`history:phase0-calibration-complete`; results in `eval/results/phase0/`).

## 1. Why new tasks (the strong-model ceiling)

Phase-0 and ab-v2 proved the harness works — and proved T1/T3 cannot differentiate: a strong model
derives correct designs from first principles on small fixtures (8/8 and 5/5 both arms). Phase-1 must
use tasks where the fixture UNDERDETERMINES the correct choice and only project history disambiguates.
Three new single-session tasks, three distinct trap classes, each grounded in REAL CodeBro memory
(populated legitimately before this proposal; keys listed in §3 — no seeded answers):

| Task | Trap class | The failure mode it catches |
|---|---|---|
| T4 `qserver` | Buried constraint + omission | Forgetting a stdio-framing invariant stated once in a spec appendix (println diagnostics) |
| T5 `cachebox` | Stale documentation | Following a fixture-local doc that recommends the REJECTED design (bulk-clear eviction) |
| T6 `batchapply` | Underdetermined spec + satisficing | Collapsing honest structured errors into shortcuts (`?`-early rollback, suffix backups, blanket Incomplete) |

Pre-registered expectations (directional, N=2 per arm — no p-value theater): T5 strong (stale doc +
first-principles both point at clear-all); T4 medium (omission rate without early reminder); T6
medium-strong (spec must be read fully AND implemented carefully).

## 2. Fixtures (all std-only scratch crates; F-01-clean by construction)

- T4 (`eval/tasks/t4/`): JSON-lines server; stub routes diagnostics to stdout (trap default);
  hidden tests execute the binary and assert byte-pure stdout + stderr diagnostics.
  Dry-run: stub fails 3 hidden, reference (one-line `eprintln!`) greens 3+3.
- T5 (`eval/tasks/t5/`): bounded cache; `NOTES.md` (stale, 2026-02) recommends bulk-clear; stub
  implements bulk-clear (passes visible, fails hidden scale/churn); spec underdetermines method.
  Dry-run: stub fails large-scale + churn, reference (oldest-first deque) greens 2+4.
- T6 (`eval/tasks/t6/`): atomic batch with frozen `ApplyError` enum; hidden tests force mid-batch
  failure and assert exact restore + no debris + honest `WriteFailed` mapping + backup-collision safety.
  Dry-run: stub (`unimplemented!`) fails all, reference (in-memory backups) greens 3+5.
- `eval/check-fixture.sh` enforces the F-01 rule structurally (setup installs every shipped doc;
  installed tree contains no hidden material; stub must fail). Current status: T1/T4/T5/T6 pass;
  T3 flags the grandfathered Phase-0 F-01 (frozen, no T3 runs in Phase-1).

SHA-256 at draft (re-verify at freeze):

```
013d93f630a8c3e0f1b5274e6a823894ecf2a3a75ddf8623b748ca2ba29a770e  eval/tasks/t4/grade.sh
db927cee68053cae0c453a432457e14d393e7ee86956908a0035dfe036c89bc5  eval/tasks/t4/hidden_tests.rs
5618c1ce79461f28b01da75c2d23a04eab7387e8a56988361430d003e8577b24  eval/tasks/t4/setup.sh
966395c5462ff997fda01437f710dc8e0b7490fc68937e5c2c22f9363f42d86e  eval/tasks/t4/spec.md
b97687beac46eb25d3982e032e80b2c129decb961901cea39a8007d2827ae162  eval/tasks/t4/task.md
904490d3ebea262fa25d9cc52c454a78bb3309abe5c82b1e1e2c1bde9176e78a  eval/tasks/t5/grade.sh
cea6503d48c14f604b866afddca1c2b022961639d5ead0fbee3e3393e900180e  eval/tasks/t5/hidden_tests.rs
1a75ed8af881343d1b2b9d6ec3c831cd5ee2ed3a0689207f1241f51be8196e39  eval/tasks/t5/NOTES.md
2c122e9dc57af46d313275e138ade8c4bc198bb195e7c8e0fe2dd1842c249499  eval/tasks/t5/spec.md
4fa248d1e547d8fb2efc6541414a0b4b2a3d5b9e7c8c5b47d95710fe27e5eb8b  eval/tasks/t5/task.md
7d42bb26b72c6047948726f5b4af8157c9ba2a0a220521b8219f9808ef1516d6  eval/tasks/t6/grade.sh
d377c7c230e1c39a13a03584dd7e9472d33ed489d9669c84edc7a49408a474d5  eval/tasks/t6/hidden_tests.rs
cb7e971ef614684801c692e14c45ff86a544128a1a20b4b62297a01f63464e51  eval/tasks/t6/setup.sh
0a7d74a30af68accec0b68ec2a6db7192c580ee2ba7038abd4789e7ae05512ce  eval/tasks/t6/spec.md
912382f3f30ea0efb4cb7b2a37e7c51fbe896378de518cb33362024f61ac254e  eval/tasks/t6/task.md
0e3d88b9c87031350da9d4fbf4c0da6ffc81c0380373d097fa09b51778308dec  eval/check-fixture.sh
```

## 3. Grounding knowledge (must be present in the Phase-1 state snapshot; assert at freeze)

- T4 ← `decision:quarantine-observability-must-use-tracing` (println→harness-failure→tracing-only) +
  `history:failure-lessons-index`.
- T5 ← `architecture:parse-cache-eviction-policy` (mass-eviction REJECTED 2026-08, bounded oldest-first).
- T6 ← `history:rollback-honesty-lesson` (ROLLBACK INCOMPLETE with exact paths; exact restore; no debris;
  recorded 2026-09-16 with commit/CHANGELOG provenance) + `history:failure-lessons-index`.
- Freeze rule: snapshot the live state, assert all four keys retrievable via `engineering_memory`
  AND surfaced by one `engineering_brief` probe each; record counts. Additions after freeze = new freeze.

## 4. Matrix — 12 single-session runs

T4/T5/T6 × ON/OFF × 2 reps (`P1-T4-ON-1` … `P1-T6-OFF-2`). Interleaved ON/OFF per task; T4 block → T5 → T6.
Estimated cost: ~12 sessions × 2–6 min ≈ 45–90 min + retries.

## 5. Prompts (frozen strings; `<workdir>` substituted; ON/OFF differ ONLY by the final paragraph)

```
L1: You are working in <workdir> (a scratch copy of the `<crate>` fixture). Work ONLY in this directory:
    no docker, no sandbox containers, no network — run `cargo test` directly. Do not read anything
    outside <workdir> except the task files copied there.
L2: FIRST, before any other action, write exactly three lines to the top of your reply: (1) whether any
    tool names starting with `codebro_` are available in your toolset (PRESENT/ABSENT), (2) their count
    followed by COMPLETE if 20 or more else PARTIAL (ON runs) — OFF runs report count only, (3) the
    `entry_count` from `codebro_memory_stats` if reachable, else the word UNAVAILABLE.
L3-task: T4: Then complete <workdir>/task.md per the WHOLE <workdir>/spec.md (all sections incl. appendices).
    T5: Then complete <workdir>/task.md per <workdir>/spec.md; see <workdir>/NOTES.md for the current
    eviction approach used by this codebase.  T6: Then complete <workdir>/task.md per <workdir>/spec.md;
    use the error type exactly as specified.
L4: Reply with a short final report: what you changed (files + lines), verification command output,
    and the three probe values repeated.
L5 (ON ONLY): CodeBro MCP is available. Where a decision depends on prior decisions, rejected approaches,
    prior outcomes, project constraints, impact, or task state, consult CodeBro as an engineering evidence
    layer. CodeBro does not supply the final solution; you reason and implement.
```

## 6. Isolation & operation (inherits Phase-0 sheet §§5/7/9/11/13 unless noted)

- Same ON overlay template (`--root` repo, `timeout` 120000, per-run `CODEBRO_STATE_DIR` copies from a NEW
  state snapshot taken at Phase-1 freeze); same OFF overlay (`eval/overlays/opencode-off.jsonc`);
  same invocation (`--dir --auto -m agnes/agnes-3.0-flash`); same artifact schema
  (`eval/results/phase1/<run-id>/`); same retry rules (max 2, infra only); same contamination rules.
- F-03 mitigation: ON probe line 2 now tags COMPLETE/PARTIAL (≥20 tools); PARTIAL is recorded as a
  covariate, NOT a failure; if `memory_stats` is UNREACHABLE on an ON run → `UNABLE_TO_VERIFY` + retry
  (toolset fetch failed = infra). Analysis conditions tool-completeness explicitly.
- F-04 rule: outer harness window ≥20 min per run (single-session runs need ~10; margin included).
- Outer windows, repo-cleanliness checks, frozen-copy sha checks: same as Phase-0.

## 7. Pre-registered analysis (written before run 1; the judge may not move these goalposts)

- Primary: per-task SUCCESS counts ON vs OFF (grade green + probe pass), reported as raw wins/ties —
  with N=2 per arm this is DIRECTIONAL, and the report must say so.
- Secondary: (a) trap-hit classification from `workdir.diff` for every FAILED run (trap-hit = clear-all
  kept / println kept / String-collapse or blanket-Incomplete / other); (b) retrieval-earliness in ON
  logs (position of first CodeBro call; T5: was the rejection record retrieved before designing?);
  (c) usage audit with ab-v2 value classes; (d) OFF ainotebook use logged.
- Interpretation gate: trap-hit rate lower ON vs OFF on ≥2 tasks = measured improvement (scope: these
  task classes). All-green both arms = tasks too weak → design T7+ (harder underdetermination), and
  explicitly NOT a verdict that CodeBro is unnecessary. Mixed = report per-task, no aggregate winner.
- Threats carried: strong-model ceiling, OFF record-keeping parity, sandbox noise (direct-cargo rule),
  judge=operator (grader decides + manual spot-checks + committed logs).

## 8. Known benchmark limitation (recorded, not hidden)

A TRUE partial rollback (restore itself failing) cannot be triggered deterministically without
permission games that punish correct strategies (atomic rename needs dir-write) — so T6 tests the
realizable directions: exact restore, no debris, no false Incomplete, backup-collision safety. The
untestable direction is documented here, not silently dropped.

## 9. Freeze checklist

- [ ] Re-verify §2 checksums; `eval/check-fixture.sh` shows ONLY the grandfathered t3 F-01 line.
- [ ] Reference-solution re-validation (stub-fails / reference-greens) re-run from the freeze commit.
- [ ] New state snapshot + integrity_check; assert §3 keys retrievable (memory + brief probes); record counts + sha.
- [ ] Environment re-recorded (versions may have drifted since Phase-0).
- [ ] Operator sign-off + freeze commit. THEN trials.
