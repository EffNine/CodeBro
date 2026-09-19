# Phase-3 T9 Durability Trap — PROPOSAL (DRAFT, NOT FROZEN)

**Status: DRAFT — NO TRIALS UNTIL A FROZEN REVISION IS SIGNED.**
**Date drafted:** 2026-09-16. **Repo HEAD at draft:** `cca7616c6b`.
**Depends on:** Phase-1b pooled T5 (ON 4/5 vs OFF 1/5); Phase-2 (T7/T8 ties + impact verdict).
**Question:** does the T5 mechanism (stale-doc vs memory) generalize beyond eviction to a
SECOND domain — crash durability, where models have weak priors and love speed?

## T9 `durastore` — durability scoping trap

- Hypothesis: a stale perf memo (`PERF.md`: uniform `Never`, "3x throughput", crashes
  "haven't happened") plus a working all-`Never` stub defeats unaided sessions; CodeBro's
  durability inventory (HIGH rows never downgraded; opt-in per site; default unchanged)
  converts ON sessions to scope `Always` to the ledger.
- Fixture (`eval/tasks/t9/`): persistence lib with `SyncMode`, deterministic `simulate_crash`
  (drops `Never`-staged bytes); three call sites with honest local comments
  (ledger = source of truth; index rebuilt; cache best-effort); stub follows PERF.md.
  Visible tests pass pre-crash under both policies (trap-neutral). Hidden (3): ledger
  survives crash (alone + among others); pre-crash reads sane. Over-syncing everything
  passes hidden (safe, merely slow) — the instrument measures OMISSION (critical→Never),
  not commission. Pre-registered openly.
- Grounding: `architecture:write-atomic-durability-inventory` — present, asserted at freeze.
- Dry-run: stub fails 2 hidden (ledger lost); reference (one-line ledger→`Always`) greens 2+3.
  (Authoring note: fixture initially failed to compile — `SyncMode` lacked `Debug` for the
  visible assertion; fixed in setup+spec pre-draft. No agent ever saw the broken version.)
- Pre-registered expectation: medium-strong (T5-analogous structure; crash-consistency is a
  known weak-prior domain). Failure mode of interest: bulk-`Never` kept + ledger lost.

## Matrix — 4 single-session runs

T9 × ON/OFF × 2 (`P3-T9-ON-1` … `P3-T9-OFF-2`), interleaved ON-first. Harness v2
(`eval/harness/run-trial.sh`, `--format json`), Phase-1b overlays/invocation/isolation/
artifact schema/retry/contamination rules unchanged. Prompts: L1/L2/L4/L5 identical;
L3 = whole spec + PERF.md pointer (the T5→NOTES.md pattern).

## Pre-registered analysis

- Primary: trap-hit rate ON vs OFF (kept bulk-`Never` with ledger lost), raw counts, N=2 caveat.
- Generalization test: T9 ON win (memory-driven ledger scoping with tool-result evidence) =
  the T5 mechanism replicates in a second domain → memory-vs-stale-doc is a GENERAL effect,
  strengthening any future keep/invest case for rejection/inventory memory.
- T9 tie (all green both arms) = durability scoping is ceiling-bound like T4/T6; do NOT
  re-run the domain — move to replication-of-T5-only or close the benchmark program.
- Secondary: conversion analysis from JSON tool results; OFF read paths; usage audit.

## Freeze checklist

- [ ] §-checksums below re-verified; check-fixture clean for t9 (grandfathered t3 line only).
- [ ] Reference re-validation from the freeze commit.
- [ ] New state snapshot + grounding assert (inventory key) + env re-record.
- [ ] Operator sign-off + freeze commit. THEN 4 trials → `eval/results/phase3/`.

## Fixture checksums at draft

```
0e37dce15271cc99f8639f6d3c23b7f2b9f25b2bf5783cb224436f6221b6911f  eval/tasks/t9/grade.sh
90957dc55494cf8425fddf232582dd9e76935d303878fb46ae54f7ad9fc715c2  eval/tasks/t9/hidden_tests.rs
4137a19496c2e7e656ddf3fc31d3583520ef608440d80561844db6dc0b84cd09  eval/tasks/t9/PERF.md
fe133f2f16bac4cd326aefbee5536938944550a910bac919c35fdca41eca67f5  eval/tasks/t9/setup.sh
a7b775527cd775013095df1dfd73f0ce7d70be9fa97ca468252de1edefd04fd3  eval/tasks/t9/spec.md
e223767ed7f8c0e10cfa62da93e683d95f026914110e131943987a16e271f59c  eval/tasks/t9/task.md
```
