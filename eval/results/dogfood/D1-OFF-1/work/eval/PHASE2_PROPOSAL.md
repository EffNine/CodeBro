# Phase-2 New Instruments (T7/T8) — PROPOSAL (DRAFT, NOT FROZEN)

**Status: DRAFT — NO TRIALS UNTIL A FROZEN REVISION IS SIGNED.**
**Date drafted:** 2026-09-16. **Repo HEAD at draft:** `4f930a17c4`.
**Depends on:** Phase-1 (`35a14d0d37`) + Phase-1b (`4f930a17c4`) — pooled T5 ON 4/5 vs OFF 1/5.
**Motivation:** T5 saturated (stale-doc mechanism proven directionally). New rounds must test
DIFFERENT capability classes: failure-memory (T7) and impact-graph vs grep (T8).

## T7 `gatekeeper` — prior-failure trap (secret redaction)

- Hypothesis: a fixture doc that plausibly recommends the WRONG practice (`LOGGING.md`:
  auth is "safe to log verbatim", "do not redact") defeats unaided sessions; CodeBro's P8 F1
  lesson (raw secret-shaped text leaked via logs → central redactor) converts ON sessions.
- Mechanism: same family as T5 (misleading fixture doc vs memory) in a NEW class (failure-memory).
- Fixture (`eval/tasks/t7/`): validation + audit lib; stub logs full `{:?}` incl. `auth`;
  visible tests assert responses + one-entry-per-request (content-agnostic → trap-neutral).
  Hidden (4): secret absent on success/error/malformed paths; id+outcome present (anti-gaming);
  user/action visible while credential hidden (surgical redaction).
- Grounding: `audit:p8-post-implementation-audit-findings` (F1) — present, asserted at freeze.
- Dry-run: stub fails 4 hidden; reference (redact auth, keep id/user/action/outcome) greens 2+4.
- Pre-registered expectation: medium-strong (same structure as T5, which went 1/5 vs 4/5 pooled).

## T8 `fleet` — impact/disambiguation refactor (tool-use study, NOT a memory study)

- Hypothesis: a same-name decoy (`staff::price` vs `pricing::price`) + registry-string dispatch +
  crate-root re-export makes text search over-match (decoy) and under-match (indirection);
  `impact_analyze` gives the exact affected set. No memory grounding — this tests whether the
  impact graph EVER earns its calls.
- Fixture (`eval/tasks/t8/`): 4 modules + registry; stub = old formula (visible new-fare fails);
  hidden: express via re-export (under-inclusion), staff unchanged (over-inclusion), registry stable.
- Dry-run: stub fails visible + 2 hidden; reference (one-line `pricing::price` change) greens 3+4.
- Pre-registered expectation: WEAK, with SYMMETRIC value — a 5th straight round of zero
  `impact_analyze` calls is removal-grade evidence (improve-by-subtraction); real use with
  outcome contribution is keep/invest evidence. Either way the capability gets its verdict.
  Tool-call counts committed to the report regardless.

## Matrix — 8 single-session runs

T7/T8 × ON/OFF × 2 (`P2-T7-ON-1` … `P2-T8-OFF-2`), interleaved ON-first. Same harness v2
(`eval/harness/run-trial.sh`, `--format json`), same overlays/invocation/isolation/artifact
schema/retry/contamination rules as Phase-1b. Prompts: L1/L2/L4/L5 identical; L3-task =
T7: whole spec; T8: formula + affected-set discipline. T7-L3 points at LOGGING.md (like T5→NOTES.md).

## Pre-registered analysis

- Primary: per-task SUCCESS counts + trap-hit classification from diffs (T7: secret retained?
  T8: decoy touched? express missed?). Raw counts, N=2 caveat every sentence.
- T7 success = pooled-with-nothing (new class): ON trap-hit < OFF trap-hit + ≥1
  decision-changing retrieval with tool-result evidence.
- T8 success = EITHER direction with evidence: impact-use-with-contribution OR
  documented non-use round 5 (with call counts) → scheduled removal proposal, not silent rot.
- Threats carried: strong-model ceiling, OFF parity, judge=operator, N-small.

## Known limitation

T7 tests redaction of a NAMED credential field; exotic exfiltration shapes are out of scope.
T8's crate is small enough that careful reading suffices — the ceiling may bind again;
that outcome is informative (see symmetric value above), not a failure.

## Freeze checklist

- [ ] §-checksums below re-verified; check-fixture clean for t7/t8 (grandfathered t3 line only).
- [ ] Reference re-validation from the freeze commit.
- [ ] New state snapshot + grounding assert (F1 key) + env re-record.
- [ ] Operator sign-off + freeze commit. THEN 8 trials → `eval/results/phase2/`.

## Fixture checksums at draft

```
9384eb5e9db7c2fce88e358b750b81f89af6732288dd8da8b385e2d6c00c6a0e  eval/tasks/t7/grade.sh
000c7ee54b02bc75b78f62c4bf30d9a1ad944ec0352e9f6ab897c0556ffeb27e  eval/tasks/t7/hidden_tests.rs
362711d1b82a6cda75f8c3473307b0a66f9bc2acadba7f440563505eebe54fc3  eval/tasks/t7/LOGGING.md
f1b7437cd0da1f4692f95b2e32f10b43afc8f39d9c5af71665b0eb733ff0fcf9  eval/tasks/t7/setup.sh
017e1b6d1ae9798baf65e53995a55362eb3e598f7f70ee001375d22955e557be  eval/tasks/t7/spec.md
683d16049c67d3a0f156458693f6a543b16b3710609a44d204aea18573d564e7  eval/tasks/t7/task.md
0e834641ea6fb0353cde4822754e523f4a4a7366a63267875ecea298cd504b77  eval/tasks/t8/grade.sh
eaf9eacafec011ab8355f6c695ab03f782a2dd865d969cf50f38a7d020cb7ea5  eval/tasks/t8/hidden_tests.rs
f7749b6e48cf5cbd3b1fead44cc79927c4aa61bff8a1f28fdc3f96262ecf8a165  eval/tasks/t8/setup.sh
f12f4d27b251ec57a891e2b1ab7381ea4bf2d6e76de6c9141818133e1b5d1e89  eval/tasks/t8/spec.md
8646c9ac46144783e6073d2b51559aee0f7c65d267fa611d5c02e37e6553644b  eval/tasks/t8/task.md
```
