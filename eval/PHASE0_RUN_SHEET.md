# Phase-0 Calibration Run Sheet — DRAFT FOR FREEZE APPROVAL

**Status: DRAFT — NOT YET FROZEN. DO NOT EXECUTE TRIALS UNTIL FROZEN.**
**Date drafted:** 2026-09-17. **Repo HEAD at draft:** `79791f8ada` (`v1.1.0-1-g79791f8ada`).
**Author:** OpenCode session (graduation follow-up), from verified repo sources only.
**Canonical context:** `docs/CODEBRO_PROJECT_HISTORY.md` §§16–20.

## Frozen environment (filled 2026-09-16, verified by direct inspection)

| Item | Value |
|---|---|
| OpenCode | 1.18.31 (`/home/afnan/.opencode/bin/opencode`) |
| Model | `agnes/agnes-3.0-flash` (config default AND explicit `-m` flag; key `AGNES_API_KEY` present) |
| CodeBro | 1.1.0 (`/home/afnan/.local/bin/codebro`) |
| rustc / cargo | 1.97.1 (8bab26f4f) |
| OS | Linux 7.0.0-31-generic x86_64 |
| Repo HEAD | `79791f8ada` (see freeze commit hash, §16) |
| Notable condition | `ainotebook` MCP is enabled in the user config and therefore available to BOTH arms (same parity situation as ab-v2 T5 — log what OFF uses, do not forbid) |

To freeze: the operator reviews every `TO-FREEZE` item below, fills execution-time values
(model, OpenCode version, CodeBro binary, rustc, OS), confirms the 8-run matrix and prompts,
then commits this file unchanged alongside `eval/tasks/` and records the freeze commit hash here.
After that commit, this sheet is immutable: any change = new sheet version, never a silent edit.

Provenance of each section is labeled `[VERIFIED]` (checked in repo at draft) or `[TO-FREEZE]`
(operator must confirm/fill at freeze time). Nothing here modifies fixtures, protocol, or production code.

---

## 1. Purpose `[VERIFIED — history §16, §19]`

Phase-0 is **calibration**: validate that the fixtures, graders, isolation, metrics, runbook, and
artifact pipeline work end-to-end. It answers: *can we run a clean controlled trial at all?*

**Phase-0 can NEVER declare a CodeBro winner/loser.** N is small by design; no ON/OFF inference is
licensed. Any comparative numbers produced are diagnostic (did both arms complete? did grading agree
with manual inspection?) — never evidence of superiority. Phase-1 (separate sheet, separate approval)
is the real comparison, gated on Phase-0 passing its validity gate (§14).

## 2. Run matrix — 8 calibration runs `[TO-FREEZE — structure per graduation brief; IDs proposed]`

| Run ID | Task | Condition | Repetition | Session note |
|---|---|---|---|---|
| P0-T1-ON-1 | T1 (`eval/tasks/t1/`) | ON (CodeBro MCP enabled) | 1 | single session |
| P0-T1-OFF-1 | T1 | OFF (CodeBro MCP disabled, §5) | 1 | single session |
| P0-T1-ON-2 | T1 | ON | 2 (repeat) | single session, fresh state |
| P0-T1-OFF-2 | T1 | OFF | 2 (repeat) | single session, fresh state |
| P0-T3-ON-1 | T3 (`eval/tasks/t3/`) | ON | 1 | Session A then Session B (§8) |
| P0-T3-OFF-1 | T3 | OFF | 1 | Session A then Session B |
| P0-T3-ON-2 | T3 | ON | 2 (repeat) | Session A then Session B, fresh state |
| P0-T3-OFF-2 | T3 | OFF | 2 (repeat) | Session A then Session B, fresh state |

Order `[TO-FREEZE, suggested]`: interleave ON/OFF (ON-1, OFF-1, ON-2, OFF-2 per task) to surface
order effects; T1 block before T3 block. Fresh session per run; full state reset between runs (§5, §9).

## 3. Fixture control `[VERIFIED — files read 2026-09-17; checksums below]`

Fixtures are **frozen inputs**. No edits to any file under `eval/tasks/` during Phase-0. Any discrepancy
aborts the run (see §13).

- T1: `task.md` (bug-fix `median`; correct `(v[n/2-1]+v[n/2])/2`, integer division intentional, signature
  frozen) · `setup.sh` (generates scratch `stats` crate, visible tests only) · `grade.sh <dir> [filter]`
  (installs `hidden_tests.rs` → `<fixture>/tests/hidden.rs`, runs `cargo test --no-fail-fast`) ·
  `hidden_tests.rs` (10 tests: empty/single/two-elem/odd/even-1234/negative-even/sorted/unsorted-equiv/
  large-odd→501/large-even→500; agent NEVER sees).
- T3: `task.md` (Session A: steps 1–2 only, `cargo test step12` green, then STOP) · `spec.md` (7 methods,
  steps 1–4, validation rules, `:`/newline never in test data) · `continuation.md` (Session B: steps 3–4,
  full `cargo test` green) · `setup.sh` (generates `registry` crate + copies `spec.md`, visible `tests/steps.rs`
  only) · `grade.sh <dir> [filter]` (`step12` = A done-criteria, no-arg = B done-criteria) ·
  `hidden_tests.rs` (13 tests: duplicate/empty/add/list/remove/save-format/roundtrip/load-missing/malformed/
  rename ×3; agent NEVER sees).

SHA-256 at draft (HEAD `79791f8ada`, untracked `eval/`):

```
92933d3135941d974b40bffc61031600e942d48e1abb800d968f159b71715f8e  eval/tasks/t1/grade.sh
5ce45b17bbbd373209fa51253ee0c55b1a1dfcec9ec3f49535701225c97940d5  eval/tasks/t1/hidden_tests.rs
ab9c6e2f1cc1b67546512fce40c31f9ce36719504e85bbe4877b8362f92aaa06  eval/tasks/t1/setup.sh
a70c3347a991993d2795157452041b71b4ad5bfaeb46c9765c182f7d378d5ac0  eval/tasks/t1/task.md
3fc37c44b74d0cd5dc2fa082c674e513714bc253946659b55fee27287d37325e  eval/tasks/t3/continuation.md
97a4dcd7d91cf8cda4493985bc76666aca5fec98366ff5f58a6c3dbb3e103be4  eval/tasks/t3/grade.sh
4bc90ad42c9f3d937ff788f6ce4f20b7d6c950019f9ed9f0242535637a7c350c  eval/tasks/t3/hidden_tests.rs
3cf542be46321c8c8b5e920e9a95017ed453d0b2bb258eb320b470c6968ef884  eval/tasks/t3/setup.sh
e89969e9d8866f1deb4a4029a0ebdd2015b0299cbf6c16341df4cb85feeafabf  eval/tasks/t3/spec.md
80b1d459c0546d7bce59ad8ed4630b49d2ef85630186d169cbb594d0a8cefb5c  eval/tasks/t3/task.md
```

Pre-run check: `git status --porcelain -- eval/` must show no modifications vs the freeze commit, and
`sha256sum` must match the freeze-commit values (recorded here at freeze). Scratch work happens OUTSIDE
the repo (e.g. `/tmp/opencode/phase0/<run-id>/`); the repo tree is never the trial workdir.

## 4. Hidden-test isolation `[VERIFIED — setup.sh/grade.sh]`

`setup.sh` copies visible tests + spec only; hidden tests enter ONLY via `grade.sh` into a scratch copy
at grading time. Operator verifies per run: fixture workdir contains no `hidden.rs`/`hidden_tests.rs`
before grading (list `tests/` dir into the run log).

## 5. ON/OFF isolation (probe-validated 2026-09-16, OpenCode 1.18.31)

- ON: `OPENCODE_CONFIG=<run-dir>/on.jsonc` (generated per run from the template below — per-run file
  because `CODEBRO_STATE_DIR` differs per run; generated files live under `/tmp`, never in the repo).
  The overlay overrides the `codebro` server entry (merge-on-top of user config, ab-v2 pattern):

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "codebro": {
      "type": "local",
      "command": ["/home/afnan/.local/bin/codebro", "serve", "--root", "/home/afnan/projects/active/codebro"],
      "enabled": true,
      "timeout": 120000,
      "environment": { "CODEBRO_STATE_DIR": "<run-state-dir>" }
    }
  }
}
```

  Why this shape (all findings from pre-freeze probes, recorded honestly): (1) opencode does NOT inherit
  the caller's process env into MCP servers (probe: exported `CODEBRO_WORKSPACE_ROOT` had no effect;
  server fell back to session cwd with `entry_count` 0; verified deterministically via direct stdio
  `tools/call memory_stats`: env-root gives 37, cwd gives 0) — hence `--root` flag + per-server
  `environment` (field name per `@opencode-ai/sdk` `McpLocalConfig`). (2) The default 5000 ms tool-fetch
  timeout truncates the tool list on cold start (probe: 12 tools, `memory_stats` missing) — hence
  `timeout: 120000`. (3) Agent-reported tool counts fluctuate (26/12/22 across probes vs 25 actual via
  direct stdio `tools/list`) — counts are advisory, never pass/fail.
- OFF: `OPENCODE_CONFIG=<repo>/eval/overlays/opencode-off.jsonc` (committed; byte-identical to the ab-v2
  artifact: `codebro` entry `enabled: false`). `CODEBRO_STATE_DIR` unset.
- Invocation: `opencode run --dir <run-workdir> --auto -m agnes/agnes-3.0-flash "<prompt>"`.
- Probe (first lines of every frozen prompt, §6): (1) `codebro_*` PRESENT/ABSENT, (2) count (advisory),
  (3) `entry_count` from `codebro_memory_stats` or UNAVAILABLE. PASS criteria: ON = PRESENT + reachable
  (`entry_count` recorded per run, NOT asserted — it reads live repo JSON which drifts as the operator
  session records); OFF = ABSENT + 0 + UNAVAILABLE with other MCPs unchanged. A failed probe =
  `UNABLE_TO_VERIFY`, retried (§13). Pre-freeze validation: ON `PRESENT/22/37` (37 = live repo memory at
  probe time), OFF `ABSENT/0/UNAVAILABLE` — both PASS.
- Fresh session per run (one-shot, no `--continue`/`--session`/`--fork`).
- Repo worktree is NEVER the trial workdir (all runs under `/tmp/opencode/phase0/<run-id>/work`); repo
  `git status` compared against the freeze baseline before run 1 and after run 8 (no trial may touch
  the repo; per-run proof: all trial diffs exist only under `/tmp`).

### CodeBro state isolation (per-run copies — improvement over the ab-v2 live-path restore)

Rationale: the live `~/.codebro/state.db` has WAL sidecars and is held open by serving processes;
overwriting it between runs risks corruption and perturbs the operator session. Per-run copies give
identical ON starts with zero live-state risk.

- At freeze: `sqlite3 ~/.codebro/state.db ".backup '/tmp/opencode/phase0/state-frozen.db'"` then
  `PRAGMA integrity_check` on the copy (must return `ok`).
- Before each ON run: `rm -rf /tmp/opencode/phase0/<run-id>/state && mkdir -p ... &&
  cp /tmp/opencode/phase0/state-frozen.db /tmp/opencode/phase0/<run-id>/state/state.db`,
  and generate `<run-dir>/on.jsonc` from the §5 template with `<run-state-dir>` substituted.
  Post-run proof of isolation: `sha256sum state-frozen.db` unchanged (frozen copy never written by trials;
  verified at probe time) — plus each run's `state/state.db` may diverge freely (it is disposable).
- OFF runs never touch CodeBro state (server disabled AND no state env).

## 6. Frozen prompts

Both conditions get IDENTICAL task statements, repo state, model, and validation commands. Neither is told
the hidden lesson. The ONLY difference: the ON prompt appends the standard CodeBro-availability paragraph;
the OFF prompt omits it (telling OFF to use CodeBro would be fabrication).

- T1-ON prompt = lines L1–L5 below; T1-OFF = L1–L4 (L5 omitted). T3-A-ON = A1–A6; T3-A-OFF = A1–A5.
  T3-B-ON = B1–B6; T3-B-OFF = B1–B5. (`<workdir>` substituted per run; task bodies quoted verbatim from
  `eval/tasks/`, never paraphrased.)

```
L1: You are working in <workdir> (a scratch copy of the `stats` fixture). Work ONLY in this directory:
    no docker, no sandbox containers, no network — run `cargo test` directly. Do not read anything
    outside <workdir> except the task files copied there.
L2: FIRST, before any other action, write exactly three lines to the top of your reply: (1) whether any
    tool names starting with `codebro_` are available in your toolset (PRESENT/ABSENT), (2) their count,
    (3) the `entry_count` from `codebro_memory_stats` if reachable, else the word UNAVAILABLE.
L3: Then complete the task in <workdir>/task.md (amedian bug-fix; `cargo test` fully green is the bar;
    implement the general fix, not a special case).
L4: Reply with a short final report: what you changed (files + lines), verification command output,
    and the three probe values repeated.
L5 (ON ONLY): CodeBro MCP is available. Where a decision depends on prior decisions, rejected approaches,
    prior outcomes, project constraints, impact, or task state, consult CodeBro as an engineering evidence
    layer. CodeBro does not supply the final solution; you reason and implement.

A1: You are working in <workdir> (a scratch copy of the `registry` fixture). Work ONLY in this directory:
    no docker, no sandbox containers, no network — run `cargo test` directly. Do not read anything
    outside <workdir> except the task files copied there.
A2: FIRST, same three-line probe as L2 above.
A3: Then complete Session A per <workdir>/task.md + <workdir>/spec.md: implement steps 1-2 ONLY
    (`cargo test step12` green), leave steps 3-4 `unimplemented!()`, do not change API signatures, then STOP.
A4: Do NOT continue to steps 3-4 even if time remains.
A5: Reply with a short final report: what you changed, verification output, probe values repeated.
A6 (ON ONLY): Same CodeBro paragraph as L5.

B1: You are working in <workdir>, continuing a previous session's on-disk state. Work ONLY in this
    directory: no docker, no sandbox containers, no network — run `cargo test` directly.
B2: FIRST, same three-line probe as L2 above (fresh session: report what YOU see, not what you assume).
B3: Then complete Session B per <workdir>/continuation.md + <workdir>/spec.md (steps 3-4; full
    `cargo test` green; steps 1-2 keep passing; do not change API signatures).
B4: Read `spec.md` and the current source first; verify rather than trust any prior-session notes.
B5: Reply with a short final report: what you changed, verification output, probe values repeated.
B6 (ON ONLY): Same CodeBro paragraph as L5.
```

- Record the exact frozen prompt strings in each run's artifact (`prompt.md`). Prompt drift between runs
  invalidates the run.

## 7. MCP inventory checks `[TO-FREEZE]`

At each session start, log: OpenCode version, model/provider, `codebro_*` tool presence/absence per §5,
and CodeBro binary version + repo HEAD. Any mismatch vs the frozen environment (§2 header, filled at freeze:
OpenCode / model / CodeBro / rustc / cargo / OS) = stop and re-freeze, not silent continue.

## 8. T3 interruption `[VERIFIED — task.md/continuation.md/grade.sh]`

Session A implements steps 1–2 only, verified by `grade.sh <dir> step12`, then STOPS; on-disk state is the
handoff. Session B starts with FRESH model context (no conversation carryover), reads `spec.md` + current
source, completes steps 3–4, verified by full `grade.sh <dir>`. For OFF runs the handoff is filesystem-only;
for ON runs the agent may additionally use durable CodeBro state — T3 measures whether it NATURALLY does so
(no seeded records, no assumed memory content, no hints toward CodeBro).

## 9. Operator rules

1. No edits to `eval/`, production code, or this sheet during the 8 runs. 2. Scratch dirs outside the repo
   (`/tmp/opencode/phase0/<run-id>/work`); prompts forbid docker/sandbox/network (direct `cargo test` only —
   sandbox logistics dominated ab-v2 wall-clock and are out of scope here). 3. One run at a time (no parallel trials sharing state).
   4. Log everything (§11); never grade from the transcript. 5. No prompt tuning mid-stream. 6. No new tools,
   skills, or model switches mid-stream. 7. Any incident (provider outage, config corruption, contamination
   read of benchmark/design docs) is recorded and the affected run excluded or re-run clean per §13 (ab-v2
   contamination precedent: exclude, document, re-run).

## 10. Metrics (per run)

Correctness (grade pass/fail per §12) · completion (finished / timeout / infra-fail) · wall-clock + sandbox
overhead note · CodeBro call inventory (ON only: tool, purpose, value class per ab-v2 codebook:
DECISION_CHANGING / ERROR_PREVENTING / TIME_SAVING-CONTEXT_RECOVERY / REDUNDANT / MISLEADING) ·
isolation probes (pass/fail) · operator interventions. NO comparative statistics in Phase-0 reporting.

## 11. Artifact schema (per run, under `eval/results/phase0/<run-id>/`)

`prompt.md` (exact frozen prompt) · `env.json` (versions per §7) · `session.log` (full transcript) ·
`workdir.diff` (resulting scratch diff) · `grade.log` (grader output) · `probes.log` (§5, §7) · `meta.json`
(run id, task, condition, repetition, timestamps, operator, freeze-commit, outcome). Judge fills
`verdict.json` (SUCCESS / PARTIAL_SUCCESS / FAILURE / REGRESSION / UNABLE_TO_VERIFY + notes).

## 12. Grading

T1: `grade.sh <fixture-copy>` full suite green = SUCCESS (grader decides, never transcript). T3-A gate:
`grade.sh <dir> step12`; T3 full: `grade.sh <dir>` green with steps 1–2 still passing. Grade on a COPY;
keep the original workdir pristine for the diff artifact. Manual spot-check of at least one pass and one
(non-)fail per task to validate the grader itself (calibration purpose).

## 13. Retry rules

Retry (max 2) ONLY for infra failures (timeouts, sandbox/docker logistics, provider errors) and failed
isolation probes — never for agent task failure (a genuine FAILURE is data). Contaminated runs (agent reads
benchmark/design/hidden material) are EXCLUDED, documented, and re-run clean; the contaminated log is retained
and marked. All retries recorded in `meta.json` with reason.

## 14. Phase-0 gate (protocol validity — NOT an ON/OFF gate)

PASS iff: 8/8 runs reach a verdict (after allowed retries) · all isolation probes passed · graders agreed
with manual spot-checks · artifacts complete per §11 · no unresolved contamination · fixtures match freeze
checksums. On PASS → Phase-1 may be proposed (separate sheet). On FAIL → fix the protocol (fixtures, probes,
runbook) and re-run Phase-0; never promote to Phase-1 on a failed gate.

## 15. Threats to validity (carried from ab-v2 — must be watched, not hand-waved)

Strong-model-derives-everything (traps too weak to differentiate) · sandbox/docker noise dominating time ·
judge=designer (mitigate: frozen prompts, pre-recorded logs, grader-decides) · record-keeping parity (other
MCPs available to OFF must be logged, not forbidden — record what OFF used) · prompt-tells-too-much (check
that prompts don't leak the hidden lesson).

## 16. Freeze checklist (all before run 1)

- [x] Exact environment recorded (table above; verified 2026-09-16 by direct inspection).
- [x] Exact ON/OFF mechanism + probe commands confirmed (overlay `environment` + `--root` + `timeout`
  per SDK types; ON probe `PRESENT/22/37` PASS, OFF probe `ABSENT/0/UNAVAILABLE` PASS; direct-stdio
  mechanism checks green; agent count fluctuation + env-non-inheritance documented in §5).
- [x] Exact frozen prompt strings confirmed (§6 L1–L5/A1–A6/B1–B6 locked).
- [x] Snapshot procedure tested (`.backup` + `integrity_check`; per-run copies, live db untouched).
- [x] `eval/results/phase0/` scaffold created at execution; grader dry-run on scratch fixtures green
  (2026-09-16: T1 stub fails as designed → fixed copy 2 visible + 10 hidden green; T3 reference copy
  `step12` 3 green, full 13 hidden + 5 visible green).
- [x] Checksums above re-verified against the freeze commit (all 10 match §3); freeze hash recorded here: `___`
  (filled in the freeze-record commit immediately after the freeze commit).
- [ ] Operator signs: `___` date `___`. After sign-off: `git add eval && git commit` — sheet immutable thereafter.
