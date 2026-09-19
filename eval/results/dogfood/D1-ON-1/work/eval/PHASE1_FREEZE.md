# Phase-1 Freeze Record (no protocol change after this point)

**Proposal:** `eval/PHASE1_PROPOSAL.md` (DRAFT §9 checklist). **Freeze commit:** this commit.
**Date:** 2026-09-16. **Operator sign-off:** user approval (`approved`).

## Corrections applied before freezing (draft review)

- `eval/PHASE1_PROPOSAL.md` §2 checksum block had a transcription error: the T5 `setup.sh` line was
  dropped and the `spec.md`/`task.md` lines carried each other's hashes (fixtures themselves untouched —
  tree was clean, blobs unchanged). Fixed in this commit; full block re-verified 17/17 below.

## Verification at freeze

- Tree clean (`git status --porcelain` empty apart from this commit's own additions, now committed).
- §2 checksums: 17/17 match (`sha256sum -c`).
- `eval/check-fixture.sh`: T1/T4/T5/T6 structurally sound (stub-fails-grading proven per task);
  sole FAIL is the grandfathered Phase-0 T3 F-01 (frozen sheet, no T3 runs in Phase-1).
- Reference re-validation (scratch, 2026-09-16): T4 3+3 green, T5 2+4 green, T6 3+5 green.
- State snapshot: `/tmp/opencode/phase1/state-frozen.db`
  sha256 `c5e3dc268dd4906c39263db7f1bb8172f53173dd3c00fd6f710e03b8a164e544`
  (`PRAGMA integrity_check` → `ok`). NOTE: byte-identical to the Phase-0 snapshot — no state.db
  writes occurred between the snapshots (interim memory writes target per-project JSON, not state.db);
  recall/history grounding therefore identical and intact.
- Grounding keys asserted retrievable at freeze:
  - `architecture:parse-cache-eviction-policy` via `engineering_memory` ✓ (verbatim rejection + direction)
  - `history:rollback-honesty-lesson` via `engineering_memory` ✓
  - `decision:quarantine-observability-must-use-tracing` via `engineering_memory` ✓
  - `history:failure-lessons-index` present in live store (direct read) ✓
  - `engineering_brief` probe (cache/eviction/diagnostics/stdout/rollback task) surfaced the println
    FAILURE outcome + reliability-gate SUCCESS outcome in its history section with provenance ✓
    (memory rank varies by keywords; agents reach specific keys via memory/recall — both verified).

## Environment at freeze (unchanged since Phase-0)

OpenCode 1.18.31 · model `agnes/agnes-3.0-flash` · CodeBro 1.1.0 · rustc/cargo 1.97.1 · Linux x86_64.
ON overlay template + OFF overlay + invocation identical to Phase-0 (proposal §6).

## Trial order

T4-ON-1, T4-OFF-1, T4-ON-2, T4-OFF-2, T5-ON-1, T5-OFF-1, T5-ON-2, T5-OFF-2,
T6-ON-1, T6-OFF-1, T6-ON-2, T6-OFF-2. Artifacts → `eval/results/phase1/<run-id>/`.
