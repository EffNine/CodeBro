# Phase-1b T5 Replication — RUN SHEET (DRAFT → frozen at sign-off commit)

**Status: DRAFT — NO TRIALS UNTIL FROZEN. Purpose:** replicate the Phase-1 T5 signal
(ON 1/2 vs OFF 0/2 with one decision-changing retrieval) at higher N, with tool-result
visibility the Phase-1 default logs lacked.
**Depends on:** Phase-1 results `35a14d0d37`; proposal `eval/PHASE1_PROPOSAL.md` (all
unchanged elements — isolation, prompts L1–L5, retry/contamination rules — inherit from it).

## Deltas vs Phase-1 (the only things that change)

1. **Scope:** T5 ONLY × ON/OFF × 3 reps = 6 runs (`P1B-T5-ON-1..3`, `P1B-T5-OFF-1..3`),
   interleaved ON-first. T5 fixtures byte-identical to Phase-1 freeze (re-verify §2).
2. **Harness v2 (committed):** `eval/harness/run-trial.sh` + `eval/harness/prompts/` (all 13
   prompt templates, byte-identical to the /tmp files consumed by Phase-0/1 runners) +
   `eval/harness/on-template.jsonc`. Sessions run with `--format json`: every `text` part
   (probe lines, reports) and every `tool_use` part (`state.status` + `state.output`) is
   preserved — usage audits read RESULTS, not just calls. Parse: probe from `text` parts;
   CodeBro retrieval from `tool_use` parts (tool name, input, output).
3. **Probe rule unchanged** (PRESENT+reachable / ABSENT+UNAVAILABLE; counts advisory;
   COMPLETE/PARTIAL tag; ON memory_stats-unreachable → UNABLE_TO_VERIFY + retry).
   Pre-freeze validation 2026-09-16: ON JSON probe `PRESENT/22/41` with completed
   `memory_stats` output in-log; OFF JSON probe `ABSENT/0/UNAVAILABLE`. Note: one bare
   ON probe transiently reported UNAVAILABLE then succeeded on the logged retry — cold-start
   flake, covered by the retry rule. Live `entry_count` drifts (37→41 across the program);
   recorded per run, never asserted.
4. **Grounding:** same four keys (T5 needs `architecture:parse-cache-eviction-policy`;
   re-asserted retrievable at freeze). NEW state snapshot at freeze (per-run copies as before).

## Pre-registered analysis (no post-hoc goalposts)

- Primary: trap-hit rate ON vs OFF **pooled** Phase-1 T5 + Phase-1b (5 arms each) AND
  round-separated; raw counts, N-small caveat in every sentence.
- Replication success = pooled ON trap-hit rate < pooled OFF rate AND ≥1 additional
  decision-changing retrieval with tool-result evidence. Anything else = instrument question
  stays open (all-green-both → trap too weak at this N/model; all-fail-both → retrieval path
  itself needs study, escalate tool-result analysis).
- Secondary: conversion analysis — for every ON run, what the retrieval returned (from log
  results) vs what the agent concluded; OFF read paths (NOTES.md? spec only?).

## Freeze record (filled at sign-off commit)

- T5 fixtures: 6/6 checksums match Phase-1 freeze. Harness hashes:
  `dee83816` run-trial.sh, `2351be00` on-template.jsonc,
  `4a811f4b` prompts/t5-on.txt, `16a7e375` prompts/t5-off.txt
  (all 13 prompt templates committed; only t5-* used this round).
- check-fixture: t5 sound; sole FAIL the grandfathered t3 F-01.
- Snapshot `/tmp/opencode/phase1b/state-frozen.db` sha256
  `ed1cbc6eba7c37b75c91603e7d2d024020f1fec622f988ec893db6aeb933b849`
  (integrity ok; differs from Phase-0/1 snapshot — live passive history writes since).
- Grounding: `architecture:parse-cache-eviction-policy` present (41 entries) + retrievable
  (natural keywords hit; one over-constrained phrasing returned empty — retrieval-sensitivity noted).
- Env (re-recorded, unchanged): OpenCode 1.18.31 · agnes-3.0-flash · CodeBro 1.1.0 · rustc/cargo 1.97.1.
- Sign-off: user `approved` (Phase-1b scope). Freeze commit: this commit.
