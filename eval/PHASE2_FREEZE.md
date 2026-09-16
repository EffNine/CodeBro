# Phase-2 Freeze Record (no protocol change after this point)

**Proposal:** `eval/PHASE2_PROPOSAL.md` (DRAFT). **Freeze commit:** this commit.
**Date:** 2026-09-16. **Operator sign-off:** user `approved`.

## Verification at freeze

- Tree clean apart from this commit's additions (4 prompt files + this record).
- Proposal §-checksums: 11/11 match. Prompt files (t7/t8 × on/off) verified:
  ON variants carry the CodeBro paragraph exactly once, OFF zero; T7 pair cites LOGGING.md.
- `eval/check-fixture.sh`: t7/t8 sound (T1/T4/T5/T6 re-verified; sole FAIL the grandfathered t3 F-01).
- Reference re-validation from this tree (scratch): T7 2+4 green, T8 3+4 green.
- State snapshot `/tmp/opencode/phase2/state-frozen.db` sha256
  `8e34dc5560ca1914e78262bee46796fc5cbf2fb9c0d0b27e1102e1e8d9a10dfe`
  (`PRAGMA integrity_check` → `ok`).
- Grounding: `audit:p8-post-implementation-audit-findings` (F1 secret-echo) retrievable with
  natural phrasing (one over-constrained phrasing returned empty — retrieval-sensitivity noted,
  consistent with Phase-1b). T8 is a tool-use study by design (no memory grounding; honest).
- Env (unchanged): OpenCode 1.18.31 · agnes-3.0-flash · CodeBro 1.1.0 · rustc/cargo 1.97.1.

## Trial order

P2-T7-ON-1, P2-T7-OFF-1, P2-T7-ON-2, P2-T7-OFF-2,
P2-T8-ON-1, P2-T8-OFF-1, P2-T8-ON-2, P2-T8-OFF-2. Artifacts → `eval/results/phase2/`.
Harness v2 (`eval/harness/run-trial.sh`, `--format json`), Phase-1b overlays/invocation/rules.
