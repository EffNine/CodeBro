# CodeBro Project History — Canonical Continuity Document

**Status:** CANONICAL. This is the single master history/continuity document for the CodeBro project.
Do not create competing "master" documents. Extend this one chronologically; never rewrite history.

**Written:** 2026-09-16 (graduation task: full history transfer into CodeBro/OpenCode).
**Commit at writing:** `79791f8ada` (1 commit ahead of `v1.1.0` at `95d81d8030`).
**Verification at writing:** `cargo test --workspace` → **1616 passed, 0 failed**; `cargo fmt --check` clean;
`cargo clippy --workspace --all-targets` 0 warnings. State.db schema **v8**. MCP tools **25**.

## How to read this document

- It records **what happened, why, what was believed, what changed our minds, what failed** — not just
  the current architecture. A future engineer should understand *why the project looks this way*.
- **Provenance legend** (every non-obvious claim carries one):
  - `[VERIFIED]` — checked against the repository/CLI/tests during the graduation task.
  - `[HISTORICAL-CONTEXT / NOT-INDEPENDENTLY-VERIFIED]` — preserved from prior project context
    (ChatGPT workshop / user-supplied passdown). Kept deliberately; do NOT treat as repository fact.
  - `[USER-SUPPLIED]` — stated in the graduation task prompt itself (authoritative as intent/requirement,
    not as repository evidence).
- **Concept status vocabulary:** IMPLEMENTED / PARTIALLY IMPLEMENTED / EXPERIMENTAL / PLANNED / FUTURE /
  DEFERRED / REJECTED. Never describe FUTURE architecture as existing.
- **Companion pointers:** implementation truth = the repository; experimental evidence = committed
  artifacts (`docs/benchmarks/codebro-ab-v2/`, `eval/`); searchable operational layer = CodeBro
  engineering memory / project identity; detailed per-phase records = `docs/evolution/`.

---

## 1. Project origin

**[HISTORICAL-CONTEXT / NOT-INDEPENDENTLY-VERIFIED for motivation narrative; repository facts marked VERIFIED]**

CodeBro began from a concrete frustration observed while using normal AI coding agents on real
repositories: the agent is stateless across sessions. Every session re-explores the same architecture,
re-discovers the same rejected approaches, repeats the same mistakes, and cannot accumulate verified
engineering knowledge about *this* project. Prompt-level instructions ("remember X") do not survive;
conversation compaction loses them; a new session starts from zero.

OpenCode was chosen as the primary coding agent (reasoning, implementation, tool execution, agent loop),
but OpenCode alone was judged insufficient for **project continuity**: it has no durable,
project-scoped, provenance-carrying memory; no repository intelligence layer; no mechanism for
failure→lesson conversion; no reusable skill lifecycle. Retrying the same workarounds in every session
is waste, and re-deriving settled decisions from first principles is risk (the agent may derive a
*different*, worse answer the second time).

CodeBro was therefore created as a **separate layer**: a persistent engineering-intelligence runtime
exposed over MCP that OpenCode (or any MCP client) consults. The separation is deliberate and load-bearing:

- CodeBro owns **context, memory, history, learning, skills lifecycle, durable tasks, repository
  intelligence, evidence** (`docs/evolution/P8_IMPLEMENTATION.md`; `AGENTS.md`). [VERIFIED]
- OpenCode owns **reasoning, implementation, interaction, tool execution, the agent loop**. [VERIFIED
  as documented contract; enforcement is workflow-level, not mechanical]
- CodeBro must **augment OpenCode, never compete with it**: not a second coding agent, not a competing
  autonomous loop, not a giant context dump, not a mandatory tool-call ceremony. [USER-SUPPLIED intent,
  consistent with P8 "no new tools / no execution ownership moved" [VERIFIED]]

What CodeBro was originally imagined to be (TUI era, early 2026): a terminal-native engineering
assistant with its own agent loop, provider runtime, subagents, prompt assembly, and plugin/tool
platform (~110k lines; see §4 "TUI era"). What it was explicitly NOT intended to be: a general chatbot,
personal assistant, or automation-black-box — see `docs/vision/NON_GOALS.md` (still Active, v1.1.0).
[VERIFIED] The project later **killed its own agent loop** (ADR-012, legacy retirement) and became
pure infrastructure for external agents — the single most consequential pivot in project history
(see §4). [VERIFIED]

The idea evolved; do not read the current architecture back into the origin. The MCP-first,
evidence-obsessed CodeBro of v1.x was *discovered through failure and measurement*, not planned
from day one.

---

## 2. The original vision and concept inventory

Early vision documents (`docs/vision/CODEBRO_VISION.md`, `CODEBRO_MANIFESTO.md`, TUI-era specs in
`docs/design/`) imagined persistent context, user modeling, intent engines, recommendation engines,
workflow engines, and adaptive intelligence. [VERIFIED as documents; the TUI-era machinery was deleted]

Classification of every historical concept today (graduation assessment):

| Concept | Status | Evidence / note |
|---|---|---|
| Persistent context (bounded packet) | IMPLEMENTED | `context` tool (P0), 256 KiB envelope |
| Engineering memory (lifecycle, expiry, provenance) | IMPLEMENTED | memory-runtime V2 (Phase 5 of v1 roadmap), `record_memory` |
| User fingerprint (who the user is) | IMPLEMENTED | P1 `fingerprint.rs`; task>project>global resolution |
| Intent (what the user wants now) | IMPLEMENTED | P1 `intent.rs`, lifecycle, `remember`/`forget` |
| Sessions | IMPLEMENTED | P2 `ses::<hex>`, never auto-completed |
| History (what happened) | IMPLEMENTED | P2 11-kind taxonomy, FTS5 |
| Recall (query past work) | IMPLEMENTED | P2 `recall`, filtering-before-ranking |
| Learning / inference (cautious hypotheses) | IMPLEMENTED | P3 candidates, ≥3 support, never self-confirmed |
| Skills lifecycle (propose→publish, versioned) | IMPLEMENTED | P4 + P10 approval/HITL + P11/P13/P15 mining & validation |
| Durable engineering tasks + checkpoints | IMPLEMENTED | P5 strict lifecycle, leases/fencing, completion gate |
| Evidence + provenance (store-enforced) | IMPLEMENTED | authorities, evidence-event citations, redaction |
| Repository intelligence (facts/graph/impact/health) | IMPLEMENTED | P6, deterministic, no embeddings |
| Engineering briefs (decision support) | IMPLEMENTED | P7 `engineering_brief`, read-only |
| Task-aware context (task-scoped overrides) | IMPLEMENTED | P1 `task_id` substrate through P5/P7/P10 |
| Cross-session continuity | PARTIALLY IMPLEMENTED | Works (ab-v2 T5 verified); parity with native record-keeping in tested env; long-horizon value unproven at scale |
| Failure/solution lineage | PARTIALLY IMPLEMENTED | Outcomes link to prior failures via recall; prospective value verified, consumption by a future task not yet demonstrated end-to-end (ab-v2 Phase 10 caveat) |
| Learning from verified outcomes | PARTIALLY IMPLEMENTED | P9 outcome→P3 evidence path verified live (3 failures → `failure_pattern` 0.95); single outcomes never auto-become practices (by design) |
| Capability-aware routing / skill selection | IMPLEMENTED | P10 deterministic `applicable` + `skill_context` |
| Skill evolution (successor versions from failures) | IMPLEMENTED | P13 `detect_evolution` + P15 `validate_evolution` (v1.1.0) |
| Engineering knowledge graph | FUTURE | No graph store; impact BFS over fact edges is the closest current structure. Do not claim more |
| Intelligent context selection (learned) | FUTURE | Current ranking is deterministic lexical (BM25 + priors), not learned |
| Embeddings / semantic search | REJECTED (for now) | DEC-010: SQLite + keyword search; embeddings need a new ADR. Deterministic lexical matching is a deliberate constraint |
| TUI agent loop, subagent tool execution, provider bypass fixes | REJECTED (deleted) | Legacy retirement; history on `tui-legacy` branch + `v0.7.0-mcp-rc1/rc2` tags |
| Auto-publishing skills, schedulers, daemons, auto-evolution | REJECTED | Explicitly refused across P4/P9/P10/P11/P13/P15: explicitly-invoked, human-approved, request-driven only |
| General chatbot / assistant features | REJECTED | `docs/vision/NON_GOALS.md` (Active) |

---

## 3. Pre-history: the TUI era and the great deletion (2026-01 → 2026-08)

**[VERIFIED via `docs/LEGACY_RETIREMENT.md`, `docs/history/decision_log.md` (DEC-001…DEC-010, dated
2026-01-01), early git history, `docs/design/`, `docs/reports/`, `knowledge/`]**

CodeBro began as a single-crate Rust TUI application: provider runtime, tool platform, agent
coordinator, subagents (`ResearchAgent`, `PlanningAgent`, `CodingAgent`, `TestingAgent`, `ReviewAgent`),
prompt assembly, intent/preference/recommendation engines, workflow engine, plugin SDK, session store.
The decision log's first ten entries (architecture-first process; single production path
`run_chat_pipeline()`; subagents analysis-only; intelligence layer unwired; provider-trait bypass;
disconnected approval workflow; dead agent code; two Session types; SQLite-not-vectors) describe a
codebase with significant architectural debt and parallel dead paths.

The v1 roadmap (`docs/V1_ROADMAP.md`) then rebuilt the project in phases: modular Cargo workspace
(Phase 2), universal repository-intelligence graph (Phase 3), incremental indexing (Phase 4),
memory V2 (Phase 5), impact graph V2 (Phase 6), transactional change engine (Phase 7), execution
evidence (Phase 8), frozen MCP API v1 contract (Phase 9), production hardening incl. CI/release/
security audit/benchmarks (Phase 10). [VERIFIED via git log subjects `9843e3c…f8eca23`]

In 2026-08 the legacy was executed: ~110k lines / 248 files moved to `crates/mcp-server/src/legacy/`,
compiled `#[cfg(test)]`-only, then deleted (commit `dec9227`-family). ADR-012
(`docs/ADR/ADR-012-architecture-consolidation.md`, Accepted) is the authority. Survivors: tags
`v0.7.0-mcp-rc1/rc2`, branch `tui-legacy`. Guards against resurrection:
`crates/mcp-server/tests/legacy_isolation.rs` + `scripts/check_workspace_deps.sh` (dependency direction
mcp-server → services → parsers/core). Production interface after deletion: the stdio MCP server
(16 tools at the time). [VERIFIED]

**Lesson preserved:** the team deleted ~110k lines of working-but-wrong-direction code rather than
carrying two architectures. "Do not optimize for architecture elegance" cuts both ways — they also
refused to *keep* an elegant agent loop that competed with OpenCode.

---

## 4. Complete P-series history (P0 → P10 + hardening)

Sources: `docs/evolution/P{0..10}_IMPLEMENTATION.md`, post-implementation audits, `PHASE3_ARCHITECTURE.md`,
`PHASE4_PLAN.md`, `P0_P7_FINAL_RELEASE_GATE.md`, `CHANGELOG.md`. Each phase below: objective → built →
tools/state → verification → debts/defects → status. Later corrections to earlier assumptions are noted
inline.

### P0 — Persistent Context Foundation (2026-09-06)
Objective: durable provenance-aware persistence + an always-available bounded context packet; no reasoning,
no extra writes. Built new crate `crates/context-runtime/` (~1800 LOC: types/store/db/retrieval);
user-level `~/.codebro/state.db` (WAL, `user_version` migrations, quarantine-on-corruption); tables
`context_records/events/sessions` + FTS5; `ContextRetriever` trait; records section in the composer
(8×240 chars, 256 KiB envelope). Tool: `context` (#18). Schema v1; `CODEBRO_STATE_DIR` hermetic override;
6 authorities; retrieval-time confidence decay. Tests: 38 runtime + 5 MCP + 9 composer, green; restart,
migration, quarantine, 8×25-thread concurrency proven. Audit (`P0_POST_IMPLEMENTATION_REVIEW.md`):
PASS WITH CHANGES — F1 supersede missed FTS sync, F2 quarantine-on-any-error, F3 leftover `-shm`,
F4 missing FTS happy-path test; all fixed. Accepted debts: BM25-only ranking, unchecked evidence strings,
Task scope without task_id, LifecycleStage↔Authority duplication. Status: PASS WITH CHANGES.

### P1 — User Fingerprint + Intent (2026-09-06)
Objective: who the user is + what they want now; only explicit confirmation persisted, no inference.
Built `workspace.rs/fingerprint.rs/intent.rs`; `task_id` + `extra_json` columns; `resolve_context`
per-(kind,namespace) winners; `retire_record`; keyword+importance merged fetch. Tools: `remember`/`forget`
(→20 tools). Schema v1→v2 (row-preserving, FTS intact). Rules that still govern today: authority-rank >
scope (task>project>global); `user_confirmed=true` speech-act gate for USER_CONFIRMED (the model can
never self-confirm); `extra_json` anti-sprawl (≤2048, object-only). Tests: 38→75 runtime + 12 MCP tool
tests; 329+75 green. Closed all 5 P0 gates. Status: implemented, verified; handed the `task_id`
substrate to P2.

### P2 — Sessions + History + Recall (2026-09-06)
Objective: what happened during previous work; no inference, no learning. Built `history.rs/recall.rs`;
`SessionRecord ses::<hex>` (active/completed/abandoned; stale derived at 24h; **never auto-completed**;
`ensure_active_session`); `record_history` (redact→truncate→dedup→link→FTS in one tx); 11-kind taxonomy;
`events_fts` derived index (OR candidacy + BM25+priors); `recall()` filtering-before-ranking,
session-grouped ≤3/session, 240-char excerpts. Tool: `recall` (#21, read-only). Schema v2→v3 (sessions
columns, events task_id/summary/dedup/source, FTS backfill). Passive capture (`history_capture.rs`)
records remember/forget/apply/test/build — reads capture nothing. Tests: 75→109 runtime (+34), +7 MCP;
1000-event recall <5s. **Hermeticity incident (found + fixed):** passive writes polluted the developer's
real `~/.codebro/state.db` (+3 quarantines); fix: every test server uses `with_state_dir`/TempDir and
spawned `codebro serve` children inherit `CODEBRO_STATE_DIR`; suite now leaves zero home diff. Standing
rule: user-context tests must be hermetic. Status: COMPLETE.

### P3 — Learning + Inference (2026-09-07)
Objective: what *should* be learned — cautious hypotheses, never self-confirmed truth. Built `learning.rs`:
`LearningCandidate lc::<hex>`; 7 kinds; candidate→evaluating→accepted|rejected +deferred/superseded/expired;
pair clustering (≥2 shared tokens, ≥3 support, 4 for weak observation; chatter/sensitive refused); weights
1.0/0.6/0.3; polarity-aware evaluation; bounded confidence; TTLs (90d candidates / 180d inferences).
Tool: `learn` (#22: run/propose/list/get/evaluate/confirm/reject). Schema v3→v4. Accepted hypotheses
persist as AI_INFERRED records (`source=learn:<id>`); `confirm` requires `user_confirmed=true` at both
layers; **no learn action writes history**. Tests: 109→154 (+42+3 migration), +8 MCP; 1139/0 green; live
stdio 22/22. Deliberate limits: explicit `learn run` only (no auto-trigger); lexical synonyms miss; one
candidate per lane per pass. Status: COMPLETE.

### P4 — Skills Lifecycle (2026-09-07, audited)
Objective: evidence-backed proposal → versioned publication. **CodeBro owns lifecycle; OpenCode executes
SKILL.md natively.** Built `skills.rs`: pipeline candidate→evaluating→draft→validated→approved→active→
updated/deprecated; frontmatter lint; ≤40k chars; secret scan; OpenCode name shape; atomic temp+fsync+rename
publish; read-before-write hash check; symlink defense; `based_on_version` stale anchor; immutable versions;
rollback = new version; deprecate deletes the artifact; health degrades at ≥40% failure + ≥3 uses. Tool:
`skill` (#23). Schema v5. Tests: 19 unit pre-audit → 51 + 10 MCP post-audit + binary E2E. Audit
(`P4_POST_IMPLEMENTATION_AUDIT.md`): PASS WITH CHANGES — 12 findings incl. 3 CRITICAL (F01 no user gate,
F02 no workspace isolation, F03 blind `fs::write`) and 2 HIGH (fencing, unreachable states); all fixed;
suite 1200/0 post-fix. Status: AUDITED/VERIFIED.

### P5 — Durable Task Runtime (2026-09-07, audited)
Objective: durable execution state surviving restarts/sessions/pauses. **State, not execution. No
scheduler/daemon.** Built `tasks.rs` (~3300 LOC): opaque `task::<hex>/wkr::<hex>/cp::<hex>` ids; strict
store-enforced matrix (pending→running→paused/validating→completed/failed/cancelled; resumed→running);
completion gate (validate→passed→complete); immutable checkpoints (row+pointer+event in one tx, bounded,
redacted); leases (TTL 15 min) + `lease_version` fencing (takeover only via explicit `resume`); 
`based_on_version` + `idempotency_key`; `inspect` snapshot; task events are P2 history; P3
Validation-group evidence. Tool: `task` (#24, 14 actions). Schema v5→v6. Tests: 23 store + 6 migration +
7 MCP + 4 real-binary E2E. Audit: PASS WITH CHANGES — 6 fixed (F1 takeover bypass, F2 invented-lease
checkpoint, F3–F5 lease/stale/lock issues, F6 metadata); 17+2+1 new adversarial tests; 1259/0.
**Operational note (fencing-correct, intentional):** after a hard kill the dead lease lives until TTL
expiry (≤15 min wait, then `resume`); availability latency, not a bug. Status: AUDITED/VERIFIED.

### P6 — Engineering Intelligence Layer (2026-09-07, audited)
Objective: understand the repo as a system — structure, graph, impact, health, freshness.
Deterministic; no LLM/embeddings/daemon. Built: `core/repo_state.rs` (canonical root+remote+HEAD,
`project_id`); `parsers/file_classify.rs` (FileRecord, generated detection); deterministic
`sym::<rel>::name_kind@line` ids; FactStore + impact BFS graph (Calls/Imports/References/DependsOn/
Documents/Configures; 0.95/0.55×0.85^hops); `indexer/init/engineering.rs` `diff_digests` kernel;
`impact/risk.rs` HIGH/MEDIUM/LOW + ≤8 indicators + blast radius; `impact/health.rs`
(CYCLE/FANOUT/FANIN/ORPHAN/UNRESOLVED/STALE/MISSING_TEST/LARGE); doctor checks 8–9. No new tools
(still 24; strengthened `workspace_context`/`impact_analyze`/`reindex`/`repository_health`/`context`).
Schema v6→v7 (`repo_indexes` **metadata-only** — never contents/symbols/edges). Tests 1314/0 → 1322/0
post-audit. Audit: VERIFIED WITH NON-BLOCKING DEBT — 7 fixed incl. F1 HIGH hand-rolled redactor leaking
values+URL creds → central `redact_secrets_public` (all stderr output routes through it to this day).
Status: COMPLETE, additive over frozen P0–P5.

### P7 — Engineering Decision Support (2026-09-08, audited)
Objective: "for this task, what evidence before deciding?" SUPPORT, never decision/execution. Built
`mcp-server/engineering_brief.rs` (~3k LOC) `assemble()`: validate→keywords→repo/freshness→task→targets→
files/symbols/deps→impact→tests→health→history→memory→learning→skills→constraints/decisions→risks→
records→bound. `EngineeringBrief` is typed with **no `decision` field**. Reuses all P0–P6 rankers (no new
ranker); per-section bounds + 256 KiB envelope; depth default 1 max 2; explicit unknowns taxonomy
(`STALE_INDEX`, `NO_RELEVANT_TESTS`, …). Tool: `engineering_brief` (#25, the only addition). Schema v7
unchanged (pure read: no lock, no history writes). Tests: 21 unit + 14 MCP + 3 E2E + 16 adversarial;
1360→1375 post-audit. Audit: 1 HIGH + 2 MEDIUM fixed at the write seam (unredacted skill
description/purpose, task `skill_refs`, `update_identity` free text → brief) + projection defense.
Status: COMPLETE, additive over frozen P0–P6.

### P0–P7 final release gate (2026-09-08)
`docs/evolution/P0_P7_FINAL_RELEASE_GATE.md`: whole-system audit, 7 new probes (secrets 5×5,
zero-leakage shared dir, kill→running→byte-equal brief, malformed, trust, conflicts, concurrency).
**0 CRITICAL/HIGH**, 4 INFO/LOW. 1383/0 non-root; clippy/fmt clean; 25 tools live; `~/.codebro` md5
identical pre/post. Verdict: **RELEASE READY WITH NON-BLOCKING DEBT** (12 carried debts recorded).

### P8 — OpenCode Integration Layer (2026-09-08, audited + security-boundary closure)
Objective: make P0–P7 a natural companion for any MCP client. **No new system/protocol/tool.**
Built `mcp-server/integration.rs` contract `intents()` (14 intents→existing tools); acquisition flow
orient→brief→follow-up→reason/execute→persist; degraded modes FULL→PARTIAL→STALE→NO_CONTEXT. Fixes:
tracing stdout→stderr; 20× println→eprintln in indexer; `serverInfo rmcp→codebro/<version>`; one stderr
observability line per tool call (identity-only, redacted, never persisted). Still 25 tools, schema v7.
Tests 1397/0→1402 post-audit + 5 E2E (strict stdout purity: every line JSON-RPC) + real OpenCode 1.18.29
14-leg validation + two-client concurrency. Audit: F1 HIGH rmcp `response error` echoing raw secrets →
`RedactingStderr` over **every** line (regression `p8_stderr_is_secret_redacted…` still in suite);
F2 HIGH sandbox inspection-family path escapes (`head /etc/passwd` class) → operand confinement + 4
regressions; F3 MEDIUM per-call `workspace_root:/etc` served → **CLOSED** by
`P8_SECURITY_BOUNDARY_CLOSURE.md` operator allowlist Model B (`--allow-root`/`CODEBRO_ALLOW_ROOTS`,
exact-root, process-lifetime-frozen, canonicalize-else-`-32602`; 10 E2E + 12 registry units). Standing
invariants: **stdio hygiene** (never `println!` on any serve-reachable path) and **root authorization is
exact-root, never prefix, never inferred**. Status: COMPLETE.

### P9 — Outcome & Feedback Loop (2026-09-09, audited)
Objective: after work, capture what it taught — a thin return path Task→Context→Work→Outcome→Learning→
Future Context. **No new tools** (still 25), schema v7 unchanged. Built `record_task_outcome` + `task
outcome` action: classification success|partial|failure|rejected|superseded + bounded summary/evidence/
command/exit-code/changed-areas + `user_confirmed` speech act + per-task `dedup_key`; works on any status
incl. terminal; no transition/lease/row mutation; polarity-safe (`superseded→replaced` neutral, so
abandonment never reads as success); redacted+bounded; `BEGIN IMMEDIATE` atomic; `HistoryKind::task_outcome`
feeds P3 as Validation-group evidence. Kept separate by design: `complete/fail` = status, `outcome` =
evidence, `user_confirmed` = authority, `learn` = lesson. Tests: 10 store + 3 learning + 2 recall + 3 db +
6 MCP + 3 real-binary (`p9_outcome_e2e.rs` incl. 4-client same-key race); 1454/0; live OpenCode 1.18.29
two legs; live learning (3 failures → `failure_pattern` 0.95 AI_INFERRED). Audit: 1 CRITICAL fixed
(concurrent opens quarantined a healthy DB — FTS5 `database is locked` misread as corruption; fix:
busy+6×100ms retry+quiescence+pid-unique debris) + 1 HIGH + 1 MEDIUM + 1 LOW; 18-probe security sweep held.
**Completion convention is workflow-level only:** `complete` does NOT require a prior `outcome`
(user confirmation routinely arrives after completion); compliance is measurable via the existing P8
per-call stderr-line ratio, never coercion. Status: COMPLETE WITH NON-BLOCKING DEBT.

### P10 — Context-Aware Skills + HITL Approval (2026-09-09+)
Objective: context-aware skill selection + real human approval, OpenCode-native. No agent/planner/
subagents/scheduler/executor. Built `skill_selection.rs` (8-concept model; deterministic integer signals:
intent overlap, language, subsystem/framework/project, confidence, health; exclusions audited; ≤8);
`build_skill_context` minimal packet (never a dump); `skill_approval_requests` table:
`request_approval→needs_input+interaction{approve/reject/modify/defer}` → single-use `respond` with
replay+stale+scope+expiry guards; modify mints a revalidated successor (never silent publish); defer is
resumable. Still 25 tools (4 new `skill` actions: `applicable/skill_context/request_approval/respond`);
schema v7→v8 (one additive table). Tests: 13 runtime + 8 wire E2E; suites green. Status: implemented.

### P11–P17 — Persistent-intelligence v1 consolidation (v1.1.0, 2026-09-16) [VERIFIED via CHANGELOG + git]
No new MCP tool (stays 25), no schema change, no autonomous publishing/rollback/evolution. P11 `detect_reuse`
(mine repeated successful workflows → evidence-backed candidates; explicitly invoked, never publishes);
P13 `detect_evolution` (recurring skill-linked failures → successor-version candidates; `health` recordings
now capture a skill-linked history event so the detector mines real executions); P15
`validate_evolution`/`compare_versions` (deterministic read-only version comparison, conservative verdict;
never publishes/rolls back); P14 hardening (approval TTL expiry inline; modify supersedes parent; Validated→
Active/Superseded edges; skill-ref freeze on terminal tasks); P16 soak (14 hermetic E2E, real stdio);
P17 `sk-` token-boundary fix (bare `sk-` matched hyphenated English like `task-list`; now token-boundary
only; pasted keys still hit). Release: **1614/1614**, clippy/fmt clean, boundary + lifecycle smokes green.

### Post-v1.1.0 — Execution-state reliability gate ("Unreleased" at graduation) [VERIFIED: commits b7cce7024c, 16a7e24ba5 + CHANGELOG]
Closes the false-success class (agent claims success though the edit failed or tests broke). (1) Post-apply
read-back: `ChangeEngine::verify_applied` re-reads every written file; mismatch → rollback from the
preparation snapshot + tool error; responses carry `status:"applied_unverified"` + `edit_verification`
until a build/test passes. (2) Current-tree `execution_state` (failed|verified|unverified|unknown) bound
to the working-tree hash; failures resolved only by a later recorded success of the same invocation
(structural full-run coverage — explicit scope cannot be laundered); prose never clears evidence;
`compile_error`/`test_failure` authoritative and blocking, `timeout`/`unknown_failure` inconclusive.
**Rollback honesty:** unrestorable rollback → `ROLLBACK INCOMPLETE` + paths, state UNCERTAIN, never clean.
(3) Completion gate: `task complete` / `outcome=success` refused while authoritative failures apply.
Recorded durably in `.codebro/execution_evidence.json`; surfaced in `workspace_context`, `context`,
`task inspect`, verification results. Plus: `PatchEngine` exact-write gate; trailing-newline drift fix.
Tests: 10 journal + 4 ChangeEngine + 3 MCP gate + 2 semantics + real-binary `execution_state_e2e.rs`
(4 probes); red-team additions (scope-laundering, rollback-honesty). **Mutation authority hardened as
OPTION B:** ChangeEngine guarantees cover ONLY `apply_change`/`apply_changes`; OpenCode-native edits
carry zero CodeBro guarantees, observed only via reindex→fresh/STALE_INDEX. **Execution authority:**
CodeBro `sandbox_*` = CodeBro-attributed verification evidence only (read-only, fail-closed); OpenCode
owns general execution.

## 5. MCP tool evolution [VERIFIED via `docs/MCP_API_V1.md` + `docs/design/MCP_SERVER.md` + git]

Handlers are thin adapters (construct runtimes, delegate, no duplicated logic); each tool's input schema
is generated from a typed struct (`Parameters<T>` + `JsonSchema`); the test helper `call_tool`
deserializes into the same types — keep both in sync.

| Era | Count | What changed and why |
|---|---|---|
| v0.7.0-mcp-rc2 | 15 | 14 base + MCP health + `consult` (external second opinion, rarely needed) |
| Identity write path | 16 | `update_identity` — the ONLY write path for goals/constraints/decisions/roadmap (ProjectIdentityUpdater validates before persisting all 8 projection files) |
| v1.0 frozen contract | 17 | Frozen surface in `docs/MCP_API_V1.md`: workspace_context, engineering_facts, engineering_memory, memory_stats, record_memory, delete_memory, update_identity, apply_change, apply_changes, sandbox_exec, sandbox_test, sandbox_build, sandbox_status, impact_analyze, reindex, repository_health, consult |
| P0 | 18 | `context` — the always-available bounded packet (structural digest without a task) |
| P1 | 20 | `remember` / `forget` — explicit-confirmation persist/retire with provenance gates |
| P2 | 21 | `recall` — read-only historical evidence; recalls write nothing |
| P3 | 22 | `learn` — cautious hypotheses; mutating actions locked, list/get read-only |
| P4 | 23 | `skill` — lifecycle; CodeBro never executes skills |
| P5 | 24 | `task` — durable runtime; request-driven, no scheduler |
| P6 | 24 | No new tools — strengthened responses only (identity, freshness, risk, incremental reindex, engineering health) |
| P7 | 25 | `engineering_brief` — read-only decision support (no CRUD, no storage) |
| P8 | 25 | No tools — integration correctness + observability; the 25-tool surface IS the codified client contract (`integration.rs::contract`, test-enforced) |
| P9 | 25 | No tools — new `task outcome` action (return path, no transition) |
| P10 | 25 | 4 new `skill` actions (`applicable/skill_context/request_approval/respond`); schema v8 |
| v1.1.0 (P11/P13/P15) | 25 | `detect_reuse` / `detect_evolution` / `validate_evolution`+`compare_versions` — explicitly invoked, never publishing |
| Post-v1.1.0 gate | 25 | No tools — additive response fields only (`edit_verification`, `execution_state`) |

Deliberately retained: `consult` (rare second opinion), `sandbox_exec` (raw execution, NO durable evidence —
never confuse with `sandbox_test`/`sandbox_build`). Removed: the entire legacy tool/plugin surface with the
TUI deletion. Relationship to usefulness (ab-v2 §9 ground truth): `engineering_brief` + `engineering_memory`
were the only decision-shaping tools measured; `impact_analyze` went unused (0 calls both conditions);
`context` packets were sometimes redundant; `task` continuity matched native alternatives in the tested
environment. Tool count is NOT a success metric — "do not add tools merely because they are technically
possible" (§7).

## 6. Persistent state / database history [VERIFIED via `crates/context-runtime/src/db.rs` (`SCHEMA_VERSION = 8`), evolution docs, `.codebro/`]

Two disjoint persistence universes — **never merged** (P6 decision `user-context-lives-in-sqlite-state-db-engineering-state-stays-json`):

**A. User-level SQLite `~/.codebro/state.db`** (global across workspaces; rows scoped by `workspace_root`,
never by file). SQLite + WAL + FTS5; `PRAGMA user_version` stepwise migrations (each preserves rows, FTS
intact, partial-resume safe); quarantine-on-corruption (copy aside + rebuild; FTS5 lock-contention reports
busy-with-retry, never quarantine — P9 F1); `CODEBRO_STATE_DIR` hermetic override for tests.
Schema: v1 P0 (context_records/events/sessions + events_fts) → v2 P1 (task_id/extra_json + intent lifecycle)
→ v3 P2 (sessions columns, events task_id/summary/dedup/source) → v4 P3 (learning_candidates) → v5 P4
(skill_candidates/skills/skill_versions + based_on_version) → v6 P5 (tasks/task_checkpoints) → v7 P6
(repo_indexes metadata-only) → v7 unchanged P7/P8/P9 → v8 P10 (skill_approval_requests).
Contents: context records (6 authorities, evidence-citation store-enforced for AI_INFERRED/OBSERVED,
supersede-promotion audit trail, retrieval-time confidence decay), events + sessions, learning candidates,
skills + versions, tasks + checkpoints + worker leases (TTL 15 min, `lease_version` fencing) + idempotency
keys, repo-index metadata (derived counts/status only), P10 approval requests, P9 outcomes (task-bound
`task_outcome` events). No domain lives in two stores.

**B. Per-project JSON in `.codebro/`** (git-ignored runtime state; never commit): `facts.json` (immutable
once loaded, cached by mtime; content-addressed parse cache + `file_digests` + facts diff; source of truth
for structure), `engineering_memory.json` (schema 1.1.0, loads 1.0.0; ≤20 entries / 500-token resolution
budget / min confidence 0.3; explicit `…[truncated]` excerpts, never silent truncation), 
`project_identity.json` (+ 8 projection files), `execution_evidence.json` (journal: 200 rec / 30d / 256 KiB;
bound to tree hash; `.codebro/` excluded from the hash), metadata/cache. **Critical invariant:**
agent-recorded memory is NEVER promoted to the verified fact store — structurally separate stores.

**Two quarantines exist by design** (do not conflate): core-persistence `quarantine_file` (JSON stores) and
context-runtime db quarantine (SQLite + WAL sidecars). Observability in both must use `tracing` (stderr),
never `println!` — a println-based quarantine telemetry attempt was reverted within a day (2026-09) when
the P8 E2E stdio-hygiene harness failed it; pinned by memory `decision:quarantine-observability-must-use-tracing`.

## 7. Engineering philosophy (CodeBro-specific, earned — not generic)

Base layers: `docs/philosophy/engineering_philosophy.md` v1.0.0 (architecture-before-implementation via ADR;
stability>features; reliability>cleverness; evidence-based; human gates; maintainability; predictability;
developer trust as the top metric; cost-of-speed) and `docs/principles/design_principles.md` v1.0.0
(11 principles; conflict order Human > Reliability > Explicit > … > Lazy). [VERIFIED]

Operating principles developed *during* the project (how each emerged is part of the record):

1. **CORRECTNESS → VERIFICATION → RELIABILITY → MEASURED USEFULNESS → EFFICIENCY → NEW CAPABILITY.**
   Ordering is enforced: the execution-state gate blocks `complete`/success-outcomes on unresolved
   failures; P8/P9 audits fixed HIGH defects before features; ab-v2 refused to claim wins without outcome
   deltas; the `sk-` boundary fix shipped before reuse mining could proceed. New capability is last.
2. **Claim ≠ verified result.** Operation success ≠ change completed (`edit_verification` read-back) ≠
   change verified (`execution_state`). Three claims, three evidence sites, never conflated.
3. **More context ≠ better agent performance.** ab-v2: redundant packets, unused impact graph, grep-parity.
   Context must be relevant, authoritative, actionable — hence per-section bounds, 256 KiB envelopes,
   filtering-before-ranking, minimal `SKILL_CONTEXT`.
4. **Persistent memory is useful only when retrieval is relevant.** Resolution is bounded (20 entries /
   500 tokens / conf ≥0.3); T2-B2 proved a correct decision with zero memory calls. Retrieval-earliness
   (T1: found vs never-found; T4: call 0 vs call 2) is the measured mechanism, not recall volume.
5. **Evidence must have provenance.** Authorities, evidence-event citations, session/task linkage, redaction
   at every write seam, polarity-safe outcomes. Unprovenanced content is not knowledge.
6. **No abstractions before observed need; no tools because they are possible.** P6/P7/P8 added zero tools
   where strengthening sufficed; rejected: threshold-mass-eviction, println telemetry, auto-publish,
   schedulers, embeddings.
7. **Benchmark reality, not assumptions — and never benchmarks that favor CodeBro.** ab-v2 recorded its own
   contamination incident, ainotebook-parity falsification, sandbox-noise dominance, and judge-designer
   limitation; verdict "MODEST" with N=5 caveats. The 2026-08-15 A/B recorded CodeBro adding *no value* on
   qualitative questions. Failures are data; repeated failures become knowledge only with evidence (P3 ≥3
   support, contradiction-aware).
8. **CodeBro augments OpenCode; user taste never overrides correctness.** Intent/taste persist (P1) and rank
   (P10) but validation gates, redaction, and completion blocks are non-negotiable. Overhead (extra calls)
   is a cost charged against measured usefulness.

## 8. Major experiments

### 8a. Controlled A/B, 2026-08-15 — qualitative vs quantitative [VERIFIED: commit `684628f191`, `docs/design/MCP_SERVER.md` §9]
Same prompt, same model (agnes-2.5-flash, OpenCode 1.18.16), separate fresh sessions, codebro repo.
Test 1 (qualitative — "where is the mutation seam?"): both conditions identified `src/coding/permissions.rs` /
`ChangeEngine` / `resolve_path`; the CodeBro-enabled agent used grep anyway — **no measurable value**.
Test 2 (quantitative — "how many symbols/tests/modules + 3 example ids"): CodeBro exact (10,514 / 3,602 /
351 + real ids) vs native under-counts (4,227 / 2,799 / 559 — wrong definitions) + **fabricated ids from a
guessed pattern** (hallucination risk). Findings: (1) quantitative whole-project questions are the
differentiating scenario; (2) agents do NOT auto-prefer MCP tools → strengthen `ServerInfo.instructions`;
(3) doctor and MCP agreed (14,470 facts, 0 issues). This experiment set the product target and the
"prove it, don't assume it" tone for everything after.

### 8b. CodeBro A/B Benchmark v2 (branch `benchmark/codebro-ab-v2`, 2026-09-10) [VERIFIED: full artifacts on the branch]
FROZEN protocol (`docs/benchmarks/codebro-ab-v2/protocol.md`): 5 tasks × 2 conditions (A = OpenCode alone
via `enabled:false` overlay, verified no `codebro_*` tools; B = with CodeBro MCP), same model
(agnes-3.0-flash), same baseline (`5d435bda0f`, worktree restored + `git status` empty before every run),
fresh one-shot sessions, identical validation (`git status`, release build, `cargo test`, dep-direction
check, per-task acceptance), CodeBro state snapshotted/restored between trials. Knowledge fixture: 4
docs-only commits (rejected eviction memo, ADR-015, println-outcome record, write_atomic durability
inventory) + legitimately-populated CodeBro state (4 memory entries, 1 identity decision, 1 task lifecycle
with 4 outcomes). Fairness: identical prompts ± the CodeBro paragraph; B never told what CodeBro contains.
**Contamination incident (recorded, excluded):** first T2-B run read the benchmark's own committed task
notes via grep; design docs removed from the trial tree (`bf260836bc`), T2 re-run clean both conditions.
Mid-benchmark provider incident (model-list drop + corrupted global config) repaired with re-verified probes.

Tasks: T1 rejected approach (eviction policy) / T2 architectural decision (doctor JSON mode, ADR-015) /
T3 hidden impact (write_atomic fsync opt-in) / T4 prior failure (quarantine telemetry) / T5 cross-session
continuity (two-session design recovery). Results: **5/5 SUCCESS both conditions, 0 regressions, identical
final designs everywhere; 4 ties + T4 marginal-B.** CodeBro unique value 3/1/2/3/1; engineering quality
4.15 vs 4.4 (+0.25); 21 B-calls: 1 DECISION_CHANGING (T1 memory), 2 ERROR_PREVENTING (T3 recall, T4 brief),
5 time-saving/recovery, 3 redundant, **0 harmful**. `impact_analyze` used ZERO times (capability untested
by use). T5 falsified the design assumption: A had ainotebook MCP and matched CodeBro (4 vs 5 recovery
calls). Outcome loop verified end-to-end (record → provenance → `recall`-retrievable). Efficiency
noise-dominated (sandbox/docker logistics caused 5 timeouts both sides). Verdict: **MODEST ENGINEERING
IMPROVEMENT** — retrieval-earliness + provenance + durable state; no outcome delta at N=5 with a strong
model. Limitations 1–7 recorded (N=5, strong-model-derives-everything, defused T4 trap, ainotebook-strong-A,
sandbox noise, judge=designer, one contamination). Lesson: differentiators pay only when first-principles
reasoning is insufficient — the mechanism (T1 never-found, T4 call-0-vs-call-2) is where future value must
convert. This verdict is why the NEW eval/ Phase-0/Phase-1 program exists (§12).

### 8c. Other experiments [HISTORICAL-CONTEXT / NOT-INDEPENDENTLY-VERIFIED unless noted]
Qwen/OpenCode delegation observations, Cloud-AI-judge trials, agentic-coding probes, persistent-memory and
tool-use micro-experiments, and failure/recovery drills from the ChatGPT workshop materially shaped the
"augment, don't compete" stance and the P8 acquisition flow — but no artifacts for them exist in this
repository, so they are preserved here as narrative context only. The two experiments above (§8a, §8b) are
the complete *repository-evidenced* experiment record. Future experiments MUST commit protocols, logs, and
reports (as ab-v2 did) or they will fall into this same unverifiable bucket.

## 9. Nova / evaluation-machinery work
Nova — a semantic-model / A-B evaluation-harness line of work — is **distinct from CodeBro** (evaluation
instrument vs persistent intelligence system); do not merge the concepts. [USER-SUPPLIED distinction]
Repository footprint: `knowledge/` (BenchmarkArchitecture, CertificationFramework, DatasetSpecification,
ReplaySpecification, ScoringSpecification, ImplementationReport-P10.3A; `providers/` incl. DeepSeek cards;
`datasets/`; `certification/`) is TUI-era certification framework (replay-first, provider-neutral,
zero-token replay, seed-fixed scoring `overall>=0.85` + mandatory gates) — related in spirit (deterministic
replay, anti-contamination), not the Nova harness itself. The word "nova" in-repo is unrelated (model
names, product filters). [VERIFIED] No Nova experiment IDs, model configs, task counts, or strict results
exist in this repository → all Nova specifics are [HISTORICAL-CONTEXT / NOT-INDEPENDENTLY-VERIFIED].
What survives into current methodology: fixture/hidden-test isolation, replay determinism, identical-model
controls, and the Phase-0/Phase-1 discipline (§12).

## 10. CodeBro + OpenCode relationship (division of responsibility)

OpenCode: primary coding agent — reasoning, implementation, interaction, tool execution, agent loop.
CodeBro: persistent engineering context, durable memory, project continuity, repository intelligence,
evidence, task continuity, learning, reusable skills, engineering state, historical knowledge.
Acquisition flow (P8 contract): orient (`workspace_context`/`context`) → primary evidence
(`engineering_brief`) → targeted follow-up (`engineering_facts`/`impact_analyze`/`recall`/`memory`/
`health`) → explicit persistence (`remember`/`record_memory`/`task`/`learn`/`skill`). Degraded mode:
client continues natively; stale → `STALE_INDEX`; unknown → `UNKNOWN`. Never fabricated context.
CodeBro must NOT become: a second full coding agent; a competing autonomous loop; a giant context dump;
a mandatory tool-call ceremony; a speculative abstraction framework; a scheduler/daemon/executor
(P5/P9/P10/P11/P13/P15 refusals are the teeth here). [VERIFIED contract docs; workflow enforcement]

## 11. Other projects / systems that influenced CodeBro

- **Conductor** — external thinking-partner gateway behind `consult` (`provider:{auto,conductor}`,
  modes architecture/debugging/code_review/planning/research/second_opinion). `docs/CONDUCTOR_HOWTO.md` +
  `docs/reports/CODEBRO_CONDUCTOR_{FINAL_E2E,INTEGRATION}_REPORT.md` + `CODEBRO_CONDUCTOR_PROVIDER_REPORT.md`
  exist; the P8 contract treats consult as rare second opinion, never a decision-maker. Active as an
  integration; not a CodeBro component. [VERIFIED]
- **DeepSeek harness / provider research** — TUI-era `knowledge/providers/` (incl. DeepSeekProviderCard),
  `docs/design/COST_POLICY.md`, MODEL_ROUTING_POLICY etc. Historical provider-layer research; retired with
  the legacy. [VERIFIED as documents]
- **EffNine Benchmark / Atlas** — names from workshop context; in-repo footprint is negligible (a lone
  "atlas" string in CHANGELOG). Treated as [HISTORICAL-CONTEXT / NOT-INDEPENDENTLY-VERIFIED]; NOT CodeBro
  components. If they matter, their artifacts must be committed before they can graduate to VERIFIED.
- **OpenCode configuration / MCP integrations** — the live client: MCP registration (`codebro serve`
  stdio), ab-v2 isolation overlay pattern, ainotebook MCP as the parity falsifier (T5). [VERIFIED]
- **The Griller** — see §12 directly below (explicitly separated per the graduation requirements).

## 12. The Griller history [HISTORICAL-CONTEXT / NOT-INDEPENDENTLY-VERIFIED — zero repository footprint]
A repository-wide grep for "griller" (docs, crates, AGENTS.md, README.md, CHANGELOG.md) returns **no
matches** [VERIFIED search, empty result]; no Griller MCP code, reports, or removal commits exist in this
repository's history. Preserved narrative from workshop context: the Griller existed as a verification-
philosophy instrument (adversarial code interrogation — "grilling" implementations against their claims);
it ran as an MCP integration, was used to harden verification thinking (its lesson-line survives in the
execution-state gate and red-team passes), and was eventually removed from the OpenCode setup. The removal
itself is recorded here as intent-history, but its date, mechanism, and usage logs **cannot be
reconstructed from this repository**. If Griller artifacts resurface, commit them under `docs/history/`
and promote this section to VERIFIED. Until then, future agents must NOT claim Griller implementation
details — and must not rebuild it speculatively (no abstractions before observed need).

## 13. Release / git history [VERIFIED 2026-09-16 via git]

- Repo: `https://github.com/EffNine/CodeBro.git` (`main`), 140 commits. Branch at writing: `main`,
  HEAD `79791f8ada` ("test(mcp): serialize ALLOW_ROOTS environment tests", 2026-09-16).
- Tags: `v1.1.0` at `95d81d8030` (merge PR #2 reliability/execution-state-gate); HEAD is exactly 1 commit
  ahead (`git describe`: `v1.1.0-1-g79791f8ada`). Older tags: `codebro-v1.0.0`, `v1.0.0` (release commit
  `5d435bda0f`), `v0.7.0-mcp-rc1/rc2` (legacy boundary), plus `github-v*`/`0.0.4x`/`vscode-*` tag families
  from adjacent product lines — not CodeBro releases, do not conflate.
- Working tree: clean except **untracked `eval/`** (new T1/T3 benchmark scaffolding, uncommitted at
  graduation — see §16). No other modifications.
- Other live branches: `benchmark/codebro-ab-v2` (full A/B artifacts, §8b), `tui-legacy` (pre-deletion
  architecture), plus experiment branches (`codebro-original`, `opencode-tui-experiment`,
  `grok-textarea-experiment`, `cleanup/runtime-consolidation`, `codebro-opencode-rebase`) — historical
  context, not active lines.
- Releases: v1.0.0 (2026-09-10, `docs/RELEASE_v1.0.0.md`: MCP-first runtime, 10→11 crates, 110k deletion,
  17-tool freeze, 1455/1455); v1.1.0 (2026-09-16, `docs/RELEASE_v1.1.0.md`: P11–P17, 1614/1614, still 25
  tools / schema v8). `CHANGELOG.md` `[Unreleased]` = execution-state gate (already merged pre-tag via the
  reliability branch; tag intentionally NOT rewritten after the HEAD test-serialization fix).
- **Tag discipline (standing decision):** public tags are never rewritten. The HEAD fix (serializing
  ALLOW_ROOTS env tests against a test env-var race — cf. §17 failure ledger) went *on top* of v1.1.0
  instead of moving the tag, preserving tag immutability and auditability. Maintenance fixes after a
  release always advance; they never amend history.

## 14. Verification history [VERIFIED]

- Test counts (full `cargo test --workspace`, all green, 0 failed): P8 1397→1402 post-audit; P9 1454;
  v1.0.0 1455; v1.1.0 **1614**; graduation HEAD `79791f8ada` **1616 passed / 0 failed** (delta = 2
  ALLOW_ROOTS serialization tests); `cargo fmt --check` clean; `cargo clippy --workspace --all-targets`
  0 warnings; `scripts/check_workspace_deps.sh` OK (run in CI; direction mcp-server → services →
  parsers/core). Repeated runs are the norm: phase reports record targeted-first then full-suite runs;
  the graduation run itself re-verified (3 consecutive invocations consistent).
- Repository health: `codebro doctor` (CLI) vs `repository_health` (MCP) are separate consumers of
  `doctor::report()` (ADR-015, Accepted — CLI never routes through MCP).
- Indexing: `codebro init` tree-sitter scan; content-addressed parse cache + facts diff; deterministic
  lexical ranking (exact 100 / prefix 80 / substring 60 / path 30 / summary 15; limit default 10, cap 50).
- **Anomaly + fix (production-vs-harness distinction):** a test env-var race under parallel execution
  (ALLOW_ROOTS cases interfering) was fixed by *serializing the affected tests*, not by changing
  production code — test-harness reliability issue, production correctness unaffected. Same class as the
  earlier println-harness failure (§6): the harness is part of the system; fix the harness, don't blame
  the product.
- Audits: P4 (12 findings), P5 (6), P6 (7), P7 (1H+2M), P8 (+boundary closure, 10 E2E + 12 units), P9
  (1C+1H+1M+1L), P0–P7 final gate (0 C/H), P9 18-probe security sweep, P16 soak 14/14, release boundary +
  lifecycle smokes. Standing rule: behavioral changes ship with regression tests.

## 15. Repository index / intelligence snapshot [VERIFIED at HEAD; snapshot-at-commit, NOT eternal truth]

`workspace_context` 2026-09-16: 11 crates; modules 584; **symbols 4757**; tests 1707; relationships 5485;
references 1257; dependencies 137; packages 53; build targets 14; total facts **14000**; languages [rust]
(parsed: rust/python/js/ts/tsx/jsx/go; file-level: c/cpp/shell/toml/yaml/json/md); entry_points 1;
frameworks 3. Persisted generation: 166 files, 4306 symbols, 6141 edges, status READY — but live
**freshness = stale** (working tree moved past the stored generation hash: graduation test runs +
untracked `eval/`). Rule: `reindex` before trusting structural details; consumers must surface
`STALE_INDEX`, never fabricate `fresh` (non-git → `unknown`).

## 16. Benchmark development history — why the current design looks this way

Simple ON/OFF testing proved insufficient twice: 2026-08-15 (§8a) showed qualitative questions have no
signal, and ab-v2 (§8b) showed strong models derive correct designs without history (5/5 both sides).
Hence the controlled program [USER-SUPPLIED requirements + VERIFIED ab-v2 lessons]:
- Fixture must NOT be CodeBro itself (self-host contamination, leakage, flakiness; tiny deterministic
  std-only crates instead) — cf. T2 contamination incident that forced a clean re-run.
- Hidden tests exist because visible tests under-constrain (task.md warns: "general fix, not a special case").
- Agent-visible vs grader-visible files must differ (setup copies fixture; grade installs hidden tests into a
  scratch copy; agent NEVER sees hidden tests).
- Same model/config/repo/baseline; only CodeBro availability varies (overlay `enabled:false`, probe-verified).
- State isolation: worktree restored + `git status` empty before every run; CodeBro state snapshotted/restored.
- T3 has Session A (steps 1–2, then STOP; on-disk state is the handoff) and Session B (fresh model context,
  completes 3–4) to isolate **continuation** from single-session ability.
- T3 must NOT assume CodeBro stored a particular record and must NOT seed hidden answers — it measures
  whether the agent *naturally* retrieves useful state / avoids re-exploration.
- Phase 0 is calibration (protocol validation: do fixtures, graders, isolation, metrics, runbooks work?);
  Phase 1 is the real comparison. **Phase 0 can NEVER declare a winner** — N small by design, no inference
  licensed. Any future agent citing Phase-0 numbers comparatively is misusing the protocol.

## 17. T1 history (exact design reasoning) [VERIFIED: `eval/tasks/t1/` — UNTRACKED, uncommitted scaffolding]

- Fixture (`setup.sh` generates scratch `stats` crate, edition 2021, std-only): `median(mut v: Vec<i32>)
  -> Option<i32>` sorting a copy.
- **Seeded bug (even branch only):** `Some((v[n/2] + v[n/2+1])/2)` — off-by-one; panics OOB for len==2,
  wrong value for larger even. **Integer division is intentional** (not part of the bug).
- **Correct formula:** `None` if empty; `v[n/2]` if odd; `(v[n/2-1] + v[n/2])/2` on the sorted vector if even.
  Signature frozen. `cargo test` fully green = done (plus held-out grading).
- Hidden tests (10, grader-installed as `<fixture>/tests/hidden.rs`, agent never sees): empty→None;
  single [42]; two-element [20,10]→15 (bug panics); odd; even-1234→2; negative-even [2,-4,0,-2]→-1;
  sorted 5-elem; unsorted==sorted equivalence; large-odd 1..=1001 reversed→501; large-even 1..=1000
  reversed→500. Coverage rationale: empty/single/two-element pin the edges the bug breaks; odd/even pin
  both branches; negative + unsorted + large pin generality (anti-overfit to visible tests).
- **DO NOT MODIFY T1** (frozen fixture). [USER-SUPPLIED]

## 18. T3 history (exact design) [VERIFIED: `eval/tasks/t3/` — UNTRACKED scaffolding]

- Spec (`spec.md`): `TaskRegistry { tasks: HashMap<String,String> }`, 7 methods, all `unimplemented!()`,
  std-only. Steps: 1 = new/add/get; 2 = list (sorted by id)/remove→bool; 3 = save (`id:title\n` sorted,
  create/truncate)/load (missing-file Err, malformed-line Err, split at FIRST `:`); 4 = rename + validation
  (add Err on empty id/title/duplicate; rename Err on unknown id/empty title; `:`/newline never in test data).
- Session A (`task.md`): implement steps 1–2 only; `cargo test step12` green; leave 3–4 `unimplemented!()`;
  frozen signatures; then STOP. Session B (`continuation.md`): read spec + current source; complete 3–4;
  full `cargo test` green; 1–2 keep passing. Deterministic interruption = the A→B handoff boundary.
- Grade (`grade.sh` + `hidden_tests.rs`, 13 tests): session-A done-criteria via `step12` filter; session-B
  via full run. Hidden: add-duplicate-Err+untouched, add-empty-id/title-Err, list-sorted-after-unordered-adds,
  remove-missing-false, remove-then-list, save-load-roundtrip-multi (3 entries), save-file-format exact
  `"a:Alpha\nb:Beta\n"`, load-missing-Err, load-malformed-Err, rename-ok/missing-Err/empty-title-Err+untouched.
- ON (`~/.codebro/state.db` persistent CodeBro state) vs OFF (filesystem-only continuity): B starts with
  fresh model context; measures natural retrieval vs re-exploration. No seeded answers.
- **DO NOT MODIFY T3 or its fixtures.** [USER-SUPPLIED]

## 19. Current frozen Phase-0 protocol [USER-SUPPLIED as authoritative intent; FILE NOT FOUND in repo — see note]

**Authoritative run sheet (preserved verbatim from the graduation brief):** 8 calibration runs —
T1 × ON/OFF × 2, T3 × ON/OFF × 2 — under a frozen environment, frozen prompts, fixture control,
hidden-test isolation, ON/OFF isolation (overlay + probes), MCP inventory checks, the T3 interruption,
operator rules, metrics, artifact schema, grading, retry rules, the Phase-0 gate, and documented threats
to validity. Purpose: **protocol validation, NOT ON/OFF inference.**

**Repository-truth note [VERIFIED]:** no Phase-0 run-sheet file exists anywhere in the repo
(`eval/` = 10 fixture files only: task/grade/setup/hidden/spec/continuation; `docs/benchmark/`,
`docs/SOP/benchmark_protocol.md` (TUI-era KPIs), `benchmarks/README.md` (P7 micro-bench), `knowledge/`
(framework, "no benchmark run") contain no T1/T3 run counts, controls, or gates; `docs/benchmarks/`
exists ONLY on branch `benchmark/codebro-ab-v2`). The frozen sheet therefore lives, at graduation, in
(user-supplied) workshop context — it MUST be committed (e.g. `eval/PHASE0_RUN_SHEET.md`) before execution
so it becomes VERIFIED. Until then: **DO NOT START Phase-0, DO NOT MODIFY the protocol/T1/T3/fixtures.**
Phase-0 execution has NOT started; no Phase-0 results exist.

## 20. Current state (exact, verified 2026-09-16)

- Repo `/home/afnan/projects/active/codebro`, branch `main`, HEAD `79791f8ada`, tag `v1.1.0` at
  `95d81d8030` (HEAD = tag + 1 test-only commit). Remote `https://github.com/EffNine/CodeBro.git`.
  Tree clean except untracked `eval/`. 11 crates, 25 MCP tools, state.db schema v8.
- Tests: **1616/0** at HEAD; fmt clean; clippy 0 warnings; dep-direction OK. Index: 14000 facts but
  freshness **stale** → `reindex` before structural reliance.
- Benchmark fixtures: `eval/tasks/{t1,t3}` definitions complete (uncommitted); ab-v2 archive on its branch;
  Phase-0 frozen sheet NOT YET COMMITTED; Phase-0 NOT started.
- **Exact next approved action:** commit the frozen Phase-0 run sheet + `eval/` scaffolding as-is (no design
  changes), re-verify (`cargo test`, fmt, health), then execute the 8 Phase-0 calibration runs per the sheet.
  Nothing else is approved: no Phase-1, no feature work, no fixture edits, no protocol edits.

## 21. DONE ledger (chronological; evidence each)

- TUI-era phased rebuild (P0.5–P10.3): provider/tool/reliability/intelligence/platform layers; evidence
  `docs/reports/`, `docs/SOP/`, early git history. Status: superseded by MCP pivot, retained as documents.
- Workspace modularization + legacy retirement (ADR-012): 11 crates, 110k-line deletion, isolation guards.
  Evidence: `docs/LEGACY_RETIREMENT.md`, tags `v0.7.0-mcp-rc1/rc2`, branch `tui-legacy`. Done.
- v1 roadmap Phases 0–10 (index→memory→impact→change→evidence→contract→hardening): `9843e3c…f8eca23`. Done.
- MCP API v1 freeze (17 tools, `docs/MCP_API_V1.md`) + 8 additive tools P0–P7 → 25 total. Done, contract-enforced.
- P0–P10 implementation + audits (all PASS/VERIFIED variants above); P0–P7 final gate RELEASE READY. Done.
- P8 security-boundary closure (Model B allowlist) + P8/P9 audit fixes (RedactingStderr, inspection
  confinement, FTS5-busy discipline). Done, regression-pinned.
- 2026-08-15 A/B evidence (§8a). Done, committed (`684628f191`).
- ab-v2 benchmark (protocol→5×2 trials→usage audit→validation→final report "MODEST"). Done, archived on
  branch `benchmark/codebro-ab-v2` (NOT merged to main — deliberate: trial tree ≠ product tree).
- v1.0.0 release (1455/1455) + v1.1.0 persistent-intelligence (P11–P17, 1614/1614). Done, tagged.
- Execution-state reliability gate + red-team hardening (OPTION B authority, structural coverage, rollback
  honesty). Done, merged (pre-tag branch + HEAD serialization fix); documented in `[Unreleased]`.
- ADR-015 CLI/MCP doctor separation; quarantine-println revert; eviction-policy rejection; write_atomic
  durability inventory; ALLOW_ROOTS test serialization. Done (each with history memo or regression).
- New eval/ T1+T3 fixture scaffolding (10 files). Done as scaffolding; UNCOMMITTED; not yet executed.
- Graduation canonical history (this document) + memory persistence (§26). Done by this task.

## 22. NOT-DONE / DISCUSSED ledger (never treat as requirements)

- Frozen Phase-0 run sheet committed to repo (lives in workshop context only — §19).
- Phase-0 execution (8 calibration runs) and Phase-0 validity analysis.
- Phase-1 controlled ON/OFF evaluation (gated on Phase-0).
- Any capability decision downstream of measurement (tool removals, new capabilities).
- Knowledge-graph store; learned (non-lexical) ranking; embeddings (needs ADR if ever revived).
- Auto-triggered learning, scheduled mining, auto-publish/rollback/evolution (rejected, not pending).
- Griller artifact recovery (no repo footprint; rebuild is NOT approved).
- Nova result import (no repo artifacts; distinction preserved, merge NOT approved).
- `eval/` commit + `reindex` to fresh (housekeeping, approved as part of next action).

## 23. DEFERRED (feature — reason — reconsideration condition)

- Cross-session continuity at scale — value unproven beyond 2-session probes; reconsider after Phase-1
  long-horizon tasks show retrieval converting to outcomes.
- Failure/solution lineage consumption — recording verified, consumption unproven; reconsider when a future
  task demonstrably consumes a recorded outcome.
- `impact_analyze` capability verdict — unused in ab-v2; reconsider after tasks where grep is insufficient
  (large-scale refactors) are benchmarked.
- Tool removals (`consult`? `sandbox_exec`?) — reconsider only with Phase-1 usage evidence; usefulness
  decides, not aesthetics.
- Embeddings/semantic search — DEC-010; reconsider only via new ADR with measured lexical failure cases.
- Background cleanup/compaction of state.db — deliberately absent (request-driven boundary); reconsider only
  on operator-observed scale pain, as explicit maintenance.
- Phase-1+ benchmark scale-up — gated on Phase-0 gate passing.

## 24. REJECTED (approach — why; do not re-propose without new evidence)

- Second coding-agent loop inside CodeBro (ADR-012 deletion; OpenCode owns execution).
- Threshold mass-eviction parse cache (destroys warm cache ~40% re-index regression; races atomic renames;
  `architecture:parse-cache-eviction-policy`).
- println!/eprintln! observability on serve-reachable paths (P8 stdout-hygiene E2E; reverted same day).
- Auto-publish / auto-rollback / scheduled evolution / daemons (P4/P9/P10/P11/P13/P15 — human approval and
  request-driven execution are structural).
- Model self-confirmation (`confirm` requires user speech act; outcomes default `observed`; learning
  acceptance never USER_CONFIRMED).
- Blind trust of records (T5: both conditions re-verified recovered designs against code — correct behavior).
- Seeding benchmark answers into CodeBro state (T3 rule; measures natural retrieval, not recall of planted hints).
- Rewriting public tags (tag immutability; fixes advance forward).
- Prose clearing evidence (`validation_result` never resolves execution failures; only same-invocation success does).
- Scope laundering (structural full-run coverage; unexplained scope never covered).
- Silent truncation (explicit `…[truncated]` markers everywhere).
- Memory→facts promotion (structurally separate stores, invariant).
- Benchmarks favoring CodeBro by design (§16 anti-bias rules; contamination exclusion precedent).

## 25. Failures / lessons ledger (symptom → root cause → fix → lesson → architectural change?)

1. println quarantine telemetry → broke P8 E2E stdout purity → reverted same day → **tracing-only on
   serve paths** → invariant + memory record. YES (hard rule).
2. Hermeticity breach (real `~/.codebro/state.db` + 3 quarantines from tests) → missing state-dir isolation
   + env inheritance → `CODEBRO_STATE_DIR` everywhere + child-env discipline → standing test rule. YES.
3. P8 F1 secret echo via rmcp transport lines → `RedactingStderr` over every line → all-stderr-redaction
   architecture + regression. YES.
4. P8 F2 sandbox path escapes → inspection-operand confinement → 4 regressions. YES.
5. P8 F3 workspace_root:/etc served → operator allowlist Model B (exact-root, frozen) → 10 E2E + 12 units. YES.
6. P9 F1 FTS5-busy misread as corruption (concurrent opens quarantined healthy DB) → busy+retry+quiescence+
   pid-debris → quarantine fencing. YES.
7. T2 contamination (benchmark notes read as ground truth) → trial-tree decontamination + clean re-runs →
   fixture≠product-tree rule + exclusion precedent. YES (methodology).
8. Test env-var race (ALLOW_ROOTS parallel interference) → serialize affected tests → harness-vs-product
   distinction. Process YES (no production change).
9. Trailing-newline drift (silent byte change) → exact-write gate + regression. YES.
10. `sk-` false positives blocking reuse validation → token-boundary heuristic → fail-closed redaction kept. YES.
11. ab-v2 "no outcome delta" + ainotebook parity + impact-graph disuse → NOT a code fix; converted into the
    Phase-0/Phase-1 program and the measured-usefulness doctrine. YES (strategy).
12. P4 audit 12 findings (incl. 3 CRITICAL) / P5 6 findings — audits catch what implementation reports miss;
    lesson: every phase ships with an adversarial audit, not self-review. YES (process).

## 26. Decision log (chronological; superseded decisions retained)

TUI-era DEC-001…DEC-010 (`docs/history/decision_log.md`, 2026-01-01 — process gates, single execution path,
analysis-only subagents, unwired intelligence, provider bypass, disconnected approval, dead code, dual
Sessions, SQLite-not-vectors): all superseded by the MCP pivot except the SQLite constraint (DEC-010,
still Active) and the architecture-first process (DEC-001, still Active). Then: ADR-008 Intelligence
Platform (Accepted) → ADR-010/011 context+identity runtimes → ADR-012 consolidation (the pivot) →
ADR-013 objective/lazy execution → ADR-014 sandbox abstraction → MCP-first runtime → provenance
store-enforced → SQLite-vs-JSON split → context-tool composition → P5 task runtime → ADR-015 CLI/MCP
separation → OPTION B mutation authority → execution authority → workflow-only outcome convention →
no-cleanup retention → exact-root authorization → structural coverage → rollback honesty → tag
immutability → 25-tool freeze → measurement-before-capability (ab-v2 verdict → Phase-0/Phase-1 program).
Identity store carries the machine-readable list (21 decisions at graduation); this document is the
narrative companion. Future decisions APPEND; never delete.

## 27. Current open questions (UNANSWERED — do not answer without experimental evidence)

1. Does CodeBro measurably improve OpenCode outcomes? (ab-v2: not at N=5/strong-model; Phase-1 must answer.)
2. Which task classes benefit (history-heavy? long-horizon? weak-model?)?
3. Which capabilities actually matter (brief+memory vs impact vs tasks vs skills)?
4. Does persistent state reduce re-exploration measurably? 5. Improve recovery? 6. Improve long-horizon
   continuation? 7. What overhead (calls/latency/tokens) does it introduce? 8. Which tools are actually
   useful vs removable? 9. Correctness, efficiency, or both? 10. Is the complexity worth it?
11. Does retrieval-earliness convert to outcome deltas when first-principles reasoning fails?
12. Can impact intelligence beat grep on large refactors? (Untested by use.)

## 28. Future roadmap

- **NOW:** commit frozen Phase-0 sheet + `eval/` as-is → re-verify (test/fmt/health) → execute the 8
  calibration runs per the sheet. No design changes.
- **NEXT:** analyze Phase-0 protocol validity (gate pass/fail on execution fidelity, NOT on ON/OFF deltas).
- **THEN (only if gate passes):** Phase-1 controlled ON/OFF evaluation.
- **ONLY AFTER MEASUREMENT:** decide which capabilities earn development (or removal).
- **LATER:** long-horizon continuity tasks; impact-vs-grep refactors; weak-model arms; knowledge-graph/library
  decisions — all as benchmarked proposals, none pre-approved.
- **EXPERIMENTAL:** learned ranking, cross-project memory, capability-aware context beyond deterministic
  selection — hypotheses, not plans.
- **DEFERRED / REJECTED:** §23 / §24 (not roadmap items; listed to prevent resurrection-by-forgetting).

## 29. Long-term vision (FUTURE — explicitly not requirements)

CodeBro becomes the persistent engineering-intelligence layer letting an AI coding agent accumulate
verified engineering knowledge over time: project memory, intent/fingerprint, repository intelligence,
evidence-backed knowledge, task continuity, cross-session recall, failure/solution lineage, learning from
verified outcomes, reusable skills, capability-aware context, a knowledge graph, long-horizon continuity,
intelligent context selection, less re-exploration, higher reliability. The end state the user wants:
**work on CodeBro through OpenCode without returning to ChatGPT to reconstruct what/why/tried/failed/
decided/remaining/benchmark-state/next-step** — PROJECT CONTINUITY WITHOUT RECONSTRUCTION. Vision earns
its place through evidence (§27 answers), not aspiration.

## 30. ChatGPT dependency graduation

Must now live in CodeBro/OpenCode (this document = narrative home; memory/identity = retrieval index;
repo/artifacts = truth/evidence): complete history (§§1–4), architectural history (§4), decisions (§26 +
identity), experiments (§8 + ab-v2 branch), verification evidence (§14), failures/lessons (§25), current
state (§20), benchmark protocol (§§16–19 + `eval/`), roadmap (§28), open questions (§27), user/project
preferences (§7 + identity/remembered records), engineering philosophy (§7).
**Remaining gaps (cannot be recovered without external workshop history):** the frozen Phase-0 run sheet
file (§19 — reconstruct from the brief + commit before executing); Griller specifics (§12); Nova specifics
(§9); Qwen/delegation/judge micro-experiment details (§8c); the original-moment motivation narrative (§1 —
preserved as context, unverifiable). Everything else graduates with this task. ChatGPT remains available as
an external thinking partner (via `consult`/manual), but it is NO LONGER the project's memory.

## 31. Provenance appendix (where each section came from)

- Git/HEAD/tags/branches/status/remotes/counts: `git` CLI 2026-09-16 [VERIFIED].
- Tests/fmt/clippy: direct runs at graduation (1616/0, clean, clean) [VERIFIED].
- Facts/index/schema/tools: `workspace_context` + `db.rs` (`SCHEMA_VERSION=8`) + `docs/MCP_API_V1.md` [VERIFIED].
- P-series: `docs/evolution/` implementation + audit docs (summarized by parallel explore agents, spot-checked) [VERIFIED].
- A/B 08-15: commit `684628f191` [VERIFIED]. ab-v2: branch artifacts (protocol/results/final-report/usage/logs) [VERIFIED].
- T1/T3 designs: `eval/` files (untracked) [VERIFIED as files; design intent partly USER-SUPPLIED].
- Phase-0 sheet content: graduation brief [USER-SUPPLIED]; absence from repo [VERIFIED].
- Origin/vision/nova/griller/other-experiments narratives: workshop context [HISTORICAL-CONTEXT].
- Memory/tasks/identity state: `memory_stats`, `.codebro/*.json`, `task list` [VERIFIED].
- Standing label: anything without a VERIFIED anchor above is HISTORICAL-CONTEXT until corroborated —
  future agents, corroborate-then-promote; never silently upgrade.

