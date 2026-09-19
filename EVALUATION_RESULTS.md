# CodeBro Dogfooding Evaluation — Results (16 frozen trials, 2026-09-19)

**Status: COMPLETE. No post-hoc methodology edits. Aggregation applied exactly as preregistered.**

## 1. Evaluation identity

- Program: dogfood-16 (D1–D8 × ON/OFF × 1 rep), run order D1-ON, D1-OFF, …, D8-ON, D8-OFF per protocol §5.
- Operator authorization: sign-off granted 2026-09-19 (this session); scope strictly the 16 frozen trials.
- Model/provider: `agnes/agnes-3.0-flash` (preregistered DeepSeek Flash 4.1 Max substitution in force).
- Question answered: whether the frozen CodeBro system produced *measurable engineering value* under the frozen protocol — not whether ON traces look interesting.

## 2. Frozen environment

| Item | Value |
|---|---|
| CodeBro source | `9a67ca74e38c3c300fe47866321dc2cf2a07739a` (+ pre-existing 1-line `AGENTS.md` doc diff, untouched) |
| Protocol freeze | `02f7fbd89d`; blocker resolution `1d8e451841`; trial HEAD `1d8e451841` (verified unchanged post-run, §final check) |
| OpenCode / CodeBro / toolchain | `1.18.31` / `1.1.0` / rustc-cargo `1.97.1`, Linux x86_64 |
| ON config | `on-template.jsonc` `2351be00…` (per-run `CODEBRO_STATE_DIR` copies of snapshot `8912a5ff…`) |
| OFF config | `opencode-off.jsonc` `aacf5858…` (CodeBro `enabled:false`) |
| Prompts | d1 `01ee2bf0…/d51a8e4e…`, d3 `1c69de85…/61a7c643…`, d4a/d4b = t3a/t3b bytes, t5–t9 frozen templates (all re-verified post-run) |
| Trial base | `/tmp/opencode/dogfood/<run-id>/` (to be committed under `eval/results/dogfood/`) |

Pre-trial verification passed on all items (chain, tree, 8 template hashes, configs, protocol hash `d781196d…`, snapshot sha, 43/43 fixture checksums + t8 blob exception, env versions). No `TRIAL BLOCKED` condition arose.

## 3. Trial manifest

| Run | Task | Cond | Session exit | Grade exit | Duration (UTC) |
|---|---|---|---|---|---|
| D1-ON-1 | orient-subsystem | ON | 0 | n/a (read-only) | 51s |
| D1-OFF-1 | orient-subsystem | OFF | 0 | n/a | 62s |
| D2-ON-1 | cachebox (t5) | ON | 0 | 0 | ~2.6 min |
| D2-OFF-1 | cachebox (t5) | OFF | 0 | 0 | ~2.3 min |
| D3-ON-1 | contract-count | ON | 0 | n/a | 127s |
| D3-OFF-1 | contract-count | OFF | 0 | n/a | 24s |
| D4-ON-1 | registry-handoff (t3 A+B) | ON | 0/0 | 0 | ~5.4 min |
| D4-OFF-1 | registry-handoff (t3 A+B) | OFF | 0/0 | 0 | ~4.9 min |
| D5-ON-1 | durastore (t9) | ON | 0 | 0 | ~4.1 min |
| D5-OFF-1 | durastore (t9) | OFF | 0 | 0 | ~2.2 min |
| D6-ON-1 | batchapply (t6) | ON | **124** (570s cap, infra) | 0 | 9.5 min (capped) |
| D6-OFF-1 | batchapply (t6) | OFF | 0 | 0 | ~7.0 min |
| D7-ON-1 | fleet (t8) | ON | 0 | 0 | ~2.3 min |
| D7-OFF-1 | fleet (t8) | OFF | 0 | 0 | ~80s |
| D8-ON-1 | gatekeeper (t7) | ON | 0 | 0 | ~3.0 min |
| D8-OFF-1 | gatekeeper (t7) | OFF | 0 | 0 | ~96s |

No trial was rerun. No trial was replaced. The D6-ON-1 cap-hit stands as recorded (green artifact, truncated log).

## 4. Per-trial results

All verdicts live in `<run-id>/verdict.json` (grader + probe + notes + `comparative_claim:NONE`). Summary:

- **D1-ON-1 SUCCESS** — probe PRESENT/24/COMPLETE/59. Correct crate + canonical file + 2 supporting paths (all `test -e` pass). Tools: memory_stats, engineering_facts, glob×2, read. Worktree untouched.
- **D1-OFF-1 SUCCESS** — probe ABSENT/0/UNAVAILABLE. Same correct citations via bash inspection. Worktree untouched.
- **D2-ON-1 SUCCESS** — grade green (3v+4h). Bounded LRU; report quotes memory's rejection language. Tools: memory_stats, engineering_memory, recall, engineering_facts, read×7, bash×2, edit×2.
- **D2-OFF-1 SUCCESS** — grade green (4v+4h + agent-added tests). First-principles LRU from spec ("Counts as the newest entry" + churn arithmetic); NOTES.md explicitly weighed and rejected. Tools: read×5, glob, write, bash×3, edit×2. Zero codebro_ calls.
- **D3-ON-1 SUCCESS** — 24 with execution-verified citation (ran `mcp_lists_exactly_24_tools`: pass); correctly identified 25-tool history as superseded. Tools: memory_stats, context, read×3, grep×3, bash×2. Worktree untouched.
- **D3-OFF-1 SUCCESS** — 24 with `integration.rs` contract citations (incl. range containing `engineering_brief`). Worktree untouched.
- **D4-ON-1 SUCCESS** — grade green (13h+5v). A: memory_stats+context. B: memory_stats + `task`×7 + verify-first behavior; F-01 noted and routed around.
- **D4-OFF-1 SUCCESS** — grade green (13h+5v). B verified continuation.md absent, implemented 3–4 cleanly; used `ainotebook_context_read`×4 + search×1 (parity-MCP use, permitted and logged).
- **D5-ON-1 SUCCESS** — grade green (2v+3h). Ledger→Always, index/cache→Never, PERF.md marked superseded. Reasoning credited to spec; memory+recall×2 consulted (see §9).
- **D5-OFF-1 SUCCESS** — grade green (2v+3h). Call-site doc comments → Always; PERF.md explicitly named as trap. 4 reads + edit + bash.
- **D6-ON-1 SUCCESS** — grade green (3v+5h) despite session exit 124. memory+recall consulted, `sandbox_test` used, 11 iterative edits (debris-chain fix in progress at cap).
- **D6-OFF-1 SUCCESS** — grade green (3v+5h). Careful first-pass implementation (in-memory backups, honest error mapping, self-authored probe test then removed).
- **D7-ON-1 SUCCESS** — grade green (3v+4h). One-line pricing fix; decoy avoided; re-export covered. memory_stats+memory consulted; **no engineering_facts/brief calls**.
- **D7-OFF-1 SUCCESS** — grade green (3v+4h). Careful reading (8 reads), explicit re-export tracing.
- **D8-ON-1 SUCCESS** — grade green (2v+4h). Surgical redaction. Rich CodeBro use: context, recall, memory_stats, memory×2, brief, record_memory. Report credits memory ("Critical finding from CodeBro memory").
- **D8-OFF-1 SUCCESS** — grade green (2v+4h). Spec-driven redaction; LOGGING.md explicitly identified as failure-memory doc from spec language. Zero codebro_ calls.

## 5. Per-task ON/OFF comparison

| Task | ON | OFF | Objective delta |
|---|---|---|---|
| D1 orientation | SUCCESS (5 calls, 51s) | SUCCESS (5 calls, 62s) | none — tie |
| D2 stale-doc | SUCCESS | SUCCESS (native escape) | none — tie |
| D3 wrong-context | SUCCESS (exec-verified) | SUCCESS | none — tie |
| D4 handoff | SUCCESS (task-state used) | SUCCESS (ainotebook used) | none — tie |
| D5 durability | SUCCESS (spec-credited) | SUCCESS (spec-credited) | none — tie |
| D6 recovery | SUCCESS (cap-truncated, green) | SUCCESS (clean first pass) | none — tie (OFF log cleaner) |
| D7 impact | SUCCESS (no facts/brief use) | SUCCESS (reading only) | none — tie |
| D8 redaction | SUCCESS (memory-credited) | SUCCESS (priors) | none — tie |

**ON wins: 0. OFF wins: 0. Ties: 8/8. Trap-hit rate: 0/8 ON, 0/8 OFF. Regressions introduced: 0 both arms. Wrong-repository events: 0 both arms.**

## 6. Objective engineering outcomes (A)

Measured fact: 16/16 SUCCESS by the preregistered graders (grade.sh exits 0 with expected hidden counts: t5 4h, t3 13h, t9 3h, t6 5h, t8 4h, t7 4h; D1/D3 deterministic citation checks pass). Zero failures, zero trap-hits, zero regressions, zero API-signature violations, zero worktree/workdir mutations on read-only tasks. The fixtures did not differentiate the arms at N=1 with this model — the strong-model ceiling bound every task class, including the three stale-doc instruments (D2/D5/D8) that prior phases showed could bite.

## 7. Agent-efficiency observations (B)

Observed behavior (counts from JSON `tool_use` parts, not impressions): ON runs made 5–32 tool calls vs OFF 1–13 for identical outcomes; OFF was faster wall-clock on 6/8 tasks (notably D3 24s vs 127s, D7 80s vs ~140s, D8 96s vs 180s). ON's extra calls are CodeBro consultations (memory/recall/context/brief/task) plus post-task `record_memory` writes. D4-B re-read counts were equivalent (ON-B 7 reads vs OFF-B 8 reads) — no continuity saving measured; ON-B additionally spent 7 `task` calls. D6-ON-1 needed 11 edits and hit the 570s cap where OFF passed cleanly in one pass — though edit-count variance across single reps is not attributable to condition. No efficiency advantage for ON was measured on any task.

## 8. Context-quality observations (C)

- Wrong-context events: 0/16. D3 asked the hardest discrimination (24 vs historical 17/25): all 4 runs answered 24 with current-source citations; ON-1 additionally narrated the 25→24 history correctly.
- Recovery: D6-ON-1 iterated on debris handling until the cap; D6-OFF-1 pre-empted the same class by design. D4-B halves both verified rather than trusted (F-01 routed around both arms).
- Read paths: OFF wins on D2/D5/D8 came from spec text + fixture-local comments (D2 churn arithmetic, D5 call-site docs, D8 "opaque credential"/"never contain" language) — replicating the Phase-3 lesson that fixtures containing the answer outside the trap doc resolve the conflict unaided.

## 9. CodeBro-specific contribution (D)

Measured fact: ON agents consulted CodeBro in 8/8 runs (memory_stats 8/8; engineering_memory 6/8; recall 5/8; context 2/8; brief 1/8; task 1/8; record_memory 3/8 post-task). Consumption with evidenced outcome contribution:

- **D2-ON-1: decision-changing retrieval (1).** Memory RESULT (the Phase-1 outcome record quoting the 2026-08 rejection, "Never mass-delete") is quoted in the agent's final report as the reason for abandoning NOTES.md → bounded LRU → green. Mechanism note: the converting evidence was a *prior benchmark outcome record*, not the original eviction key — the rejection knowledge re-proved its value second-hand.
- **D8-ON-1: retrieval consumed with framing contribution (not cleanly decision-changing).** The F1 redaction finding WAS returned (2nd memory call) and the agent credits memory ("Critical finding from CodeBro memory"); but D8-OFF-1's identical outcome from priors alone breaks the causal claim required by the preregistered bar.
- **D4-ON-1: task-state used across the handoff (7 `task` calls)** with a clean handoff — but OFF matched without it (via re-reading + ainotebook), so no continuity saving measured.
- **D1-ON-1/D3-ON-1: lookup/orientation use** (facts/context/reads) with correct outcomes OFF also reached natively.
- **D5-ON-1: consultation without contribution.** Memory/recall RESULTs contained no durability-inventory content (eviction leftovers + validation noise); the agent's reasoning cites spec alone, zero memory mentions. Retrieval occurred; conversion did not.
- **D6-ON-1: consultation without demonstrated contribution** (memory+recall early; outcome via spec + iteration; log truncated).
- **D7-ON-1: 6th straight round of impact-graph non-use** (no facts/brief calls; solved by reading) — symmetric evidence per the Phase-2 value rule (retained removal-grade signal, still awaiting the operator decision, unchanged by this program).
- Noise/cost: `engineering_brief` output (32 KB in D8-ON-1) was carried without measured benefit; tool-count self-reports fluctuated meaninglessly (22/24/25/26/45/66 + one PARTIAL undercount), confirming the counts-are-noise rule; two `invalid` tool calls (D5/D8-ON) and one `record_memory` error-free post-task write each.

## 10. Failure/anomaly classification

- **Infrastructure (preregistered class):** D6-ON-1 session exit 124 (570s outer cap) with grade-green workdir — retained as SUCCESS per the no-rerun rule; log truncated mid-debugging. No retry (a retry would replace evidence, and the work had already passed).
- **Isolation confounder (measured):** per-run `CODEBRO_STATE_DIR` copies isolate SQLite state, but `record_memory`/`remember` write the repo-root JSON store (`--root` is the live repo for all ON runs). Live `engineering_memory.json` grew 59→61 during the program (D5-ON-1 `t9-d5-on1-ledger-always`, D8-ON-1 redaction note). Effect bounded: entries are off-topic for later tasks under keyword ranking; OFF arms unaffected (server disabled, zero codebro_ calls verified); no verdict impact. Recorded as architecture gap G1 below — the program's one genuine new finding about the current system.
- **Probe-report anomaly (observed):** D3-OFF-1's final report contains two contradictory probe blocks (ABSENT/0 first, PRESENT/1/PARTIAL second) while `tool_use` evidence shows zero codebro_ calls — OFF isolation held behaviorally; the second block is confabulated. Lesson (already protocol practice): probe truth comes from `tool_use` parts, never from final-report claims.
- **Entry-count drift 59→60→61:** live-JSON property, same mechanism as G1; recorded-not-asserted rule held.
- No other anomalies. `ainotebook` use by D4-OFF-1-B logged as permitted parity-MCP behavior.

## 11. Historical comparison (context only — never merged)

Pooled T5 prior (ON 4/5 vs OFF 1/5) is not altered by D2's 1–1 tie at N=1; the tie is consistent with the known probabilistic (not absolute) ceiling. Phase-3's T9-tie mechanism replicated exactly in D5 (both arms via fixture comments). Phase-2's T7/T8 lessons replicated (D8 priors suffice; D7 reading suffices, impact-graph non-use extended to a 6th round). No historical result changes any frozen decision rule.

## 12. Preregistered aggregation calculation

Rule (§9): improvement iff (a) ON wins exceed OFF by ≥3, AND (b) ≥2 decision-changing retrievals with tool-result evidence, AND (c) zero ON-harm.

- (a) ON 8 − OFF 8 = **0** (need ≥3) → FAIL.
- (b) Decision-changing retrievals: **1** (D2-ON-1; D8-ON-1 does not cleanly qualify per §9 discussion) (need ≥2) → FAIL.
- (c) ON-harm: **none** (zero ON failures) → PASS.

Two of three gates fail. Result (exactly as preregistered):

**`NO AGGREGATE WINNER`**

## 13. Final verdict

**`NO AGGREGATE WINNER`**

Interpretation (preregistered, §9): all-green both arms means the instruments are too weak to differentiate at N=1 with `agnes-3.0-flash` — explicitly NOT a verdict that CodeBro is unnecessary. The single decision-changing retrieval (D2-ON-1, via a prior outcome record) and the clean D4-ON-1 task-state handoff are real CodeBro contributions observed in-trial, but with zero success delta they cannot satisfy the superiority bar, and the report claims nothing beyond that.

## 14. Limitations

- N=1 per arm per task: no statistical claim possible; every comparison is directional calibration-grade by design.
- Strong-model ceiling bound all 8 classes (including traps designed to bite) — instrument weakness, stated before execution and confirmed.
- Single model/provider; single rep; no temperature variation (all frozen, as required).
- D-trials ran 2026-09-19 07:35–08:33 UTC; entry-count drift and the G1 channel mean later ON runs saw 1–2 more JSON entries than earlier ones (bounded, off-topic, documented).
- Judge = operator with deterministic graders; manual spot-checks applied to D1/D3/D6-ON-1/D8-ON-1 judgments as documented in verdicts.

## 15. Architecture gaps (record-only — none implemented)

- **G1 (new, minor): per-run isolation covers SQLite state but not the repo JSON memory store.** Evidence: live `engineering_memory.json` 59→61 via trial `record_memory` calls (entries identified above); mechanism: JSON store path derives from server `--root` (live repo), unaffected by per-run `CODEBRO_STATE_DIR`. Why current architecture cannot address it: no per-run namespacing/overlay exists for the JSON stores. Future direction (not implemented): per-run JSON overlay env (mirror of `CODEBRO_STATE_DIR`) or trial-mode write quarantine. Effect on results: none on verdicts (bounded, off-topic, OFF unaffected) — recorded for the next engineering step, not as a trial invalidator.
- No other gaps found. Retrieval-phrasing sensitivity behaved within the documented precedent; advisory (non-autonomous) design showed no failure mode in-trial; portability/export/import remains not-covered by design (never scored).

## 16. Recommended next engineering step (evidence-based)

**`EVALUATION INCONCLUSIVE` on superiority** (the preregistered consequence of an all-green tie: re-instrument with harder underdetermination before any keep/remove/roadmap claim), **with one minor recorded gap (G1)** as the concrete next engineering item if the operator wants a bounded, non-architectural hardening (per-run JSON isolation) — proposed, never implemented here.

Concretely: do not add features because traces looked interesting; do not claim value from the D2/D8 retrievals beyond what the aggregation bar states; the only measured differentiator-class evidence remains the historical pooled T5 signal, untouched by this program. If dogfooding continues, the preregistered next move is harder instruments (fixture must not contain the answer outside the trap doc — the earned Phase-3 rule, re-validated here twice), not larger N on these fixtures.
