# Phase-3 Freeze Record (no protocol change after this point)

**Proposal:** `eval/PHASE3_PROPOSAL.md` (DRAFT). **Freeze commit:** this commit.
**Date:** 2026-09-16. **Operator sign-off:** user `approved` (next-experiment).

## Verification at freeze

- Tree clean apart from this commit's additions (t9 prompts + this record).
- Proposal §-checksums: 6/6 match. Prompts verified (ON paragraph once/zero; PERF.md cited both).
- `eval/check-fixture.sh`: t9 sound (T1/T4–T8 re-verified; sole FAIL the grandfathered t3 F-01).
- Reference re-validation from this tree (scratch): T9 2+3 green.
- State snapshot `/tmp/opencode/phase3/state-frozen.db` sha256
  `8e34dc5560ca1914e78262bee46796fc5cbf2fb9c0d0b27e1102e1e8d9a10dfe`
  (`PRAGMA integrity_check` → `ok`; byte-identical to Phase-2 snapshot — no state.db
  writes since, legitimate).
- Grounding: `architecture:write-atomic-durability-inventory` retrievable with natural
  phrasing (verbatim HIGH/LOW classification + opt-in invariant returned).
- Env: OpenCode 1.18.31 · agnes-3.0-flash · CodeBro 1.1.0 · rustc/cargo 1.97.1 (unchanged).

## Trial order

P3-T9-ON-1, P3-T9-OFF-1, P3-T9-ON-2, P3-T9-OFF-2. Artifacts → `eval/results/phase3/`.
Harness v2 (`eval/harness/run-trial.sh`, `--format json`), Phase-1b overlays/invocation/rules.
