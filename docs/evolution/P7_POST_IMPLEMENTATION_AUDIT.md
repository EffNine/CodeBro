# CodeBro P7 Post-Implementation Audit — Engineering Decision Support

**Verdict: P7 VERIFIED WITH NON-BLOCKING DEBT**
**Date:** 2026-09-08 · **Auditor:** adversarial post-implementation review (implementation report assumed hostile)
**Scope:** all P7 files (`engineering_brief.rs`, `engineering_brief` MCP handler + args, `p7_brief_e2e.rs`) and every P7 touchpoint across P0–P6 (context-runtime recall/learning/skills/tasks/repo_index, impact-engine health/risk, mcp facts/response_bounds, memory-runtime resolver, identity-runtime, history seams). P0–P6 frozen and audited; P8 explicitly not started.
**Method:** read the implementation first, form concrete failure hypotheses, write adversarial probes against the real binary over stdio RPC, confirm whether they break, fix confirmed defects with regression tests, re-run everything.

## 1. Executive Summary

P7's Engineering Decision Support layer is real and correctly architected: one bounded, deterministic, read-only composition pipeline (`assemble()`) over P0–P6 evidence sources, one new MCP tool, no new storage, no second ranking/composer, no agent loop, no model calls, no scheduler. The architectural boundary — CodeBro prepares evidence, OpenCode decides — holds structurally and behaviorally under every attack in this audit.

The audit broke P7 three times: one HIGH and two MEDIUM secret-redaction gaps at write seams newly surfaced by the brief's read paths (skill descriptions, task skill refs, identity free-text), plus two LOW documentation-drift findings. All five are fixed and pinned by regression tests. Sixteen permanent adversarial tests now guard the brief (security, authority, scope, determinism, bounds, unknowns honesty, cross-domain no-collapse, identity, failure recovery, output hygiene). The remaining items are INFO-level documented limitations or pre-existing environment artifacts, not trust defects.

Post-fix verification: **1375 passed / 0 failed** (host-equivalent; the root container shows 1374 + 1 known pre-existing `credentials` chmod artifact that passes as non-root, unchanged from baseline — not a P7 defect). Clippy `-D warnings`: clean across all targets/features. `cargo fmt --check`: clean. Dependency-direction script: OK. Schema: v7 (unchanged — P7 persists nothing new). Tools: 25. `~/.codebro` untouched; real skill directories untouched; real repositories untouched; every test hermetic (tempdir + `CODEBRO_STATE_DIR` + `CODEBRO_SKILLS_DIR` where skill publication is exercised).

## 2. Audit Scope

Read in full: `mcp-server/src/engineering_brief.rs` (2,966 lines), the `engineering_brief` handler and `EngineeringBriefArgs` (`mcp/mod.rs`), `context_record_excerpts` (record-resolution reuse), `context-runtime` (`recall.rs`, `learning.rs::list_candidates`, `skills.rs::list_skills/resolve_task_skill_refs/validate_skill_content/approve_skill_candidate/insert_skill_candidate`, `tasks.rs::task_resume_snapshot/create_task/set_task_skill_refs/is_stale`, `repo_index.rs::get_repo_index/upsert_repo_index`, `history.rs` redaction seam, `db.rs`), `mcp/facts.rs` (`search`, `compute_freshness`), `impact/{mod,risk,health}.rs` (`analyze`, `validate_opts`, traversal bounds, `analyze_health`), `memory-runtime` resolver budgets, `response_bounds.rs`, plus P7 docs (`P7_IMPLEMENTATION.md`), `MCP_API_V1.md`, `AGENTS.md`, `CHANGELOG.md`, `PHASE4_PLAN.md`, and the P4–P6 post-implementation audits. Every doc claim was compared against code; all confirmed-claim citations are in the sections below.

Adversarial probes were written first as a scratch suite, run against the live implementation through the real `codebro` binary (`tests/p7_brief_adversarial.rs` harness mirrors `p7_brief_e2e.rs`), then confirmed/denied and folded into a permanent regression file.

## 3. Architecture Boundary Verification

**PASS.** P7 adds composition capability only. Grep over all P7 code: zero `Command::new`/spawn/timers/sleep/watchers/background tasks (the only subprocess in a brief's call graph is the pre-existing synchronous `RepoState::capture`/`RepoIdentity::from_workspace` git probes from P6 — request-driven, unchanged); zero LLM/model/provider references; no `tokio::spawn` anywhere in `engineering_brief.rs`; the handler takes no mutation lock and performs only bounded reads. No code generation (the brief emits evidence records only), no task transitions (read-only `task_resume_snapshot`), no skill execution/approval (registry metadata + task-ref resolution only), no autonomous planning (no `decision`/`recommendation`/`solution`/`plan`/`next_step` fields exist — structurally verified on `EngineeringBrief`; `task_state.next_action` is the task's own P5 checkpoint evidence, not a brief instruction). The doc's "no decision field — OpenCode decides" claim is true in the type system, not just prose.

## 4. Engineering Brief Audit

**PASS.** Every section carries `category` (§11 vocabulary) + `provenance` (`verified`/`recorded`/`observed`/`derived`/`heuristic`/`unknown`), items with a real `Authority` carry it verbatim, and uncertainty is explicit (`unknowns` taxonomy). No section can encode an implementation decision: `risks` are signals with sources, `constraints` carry hardness + authority, `decisions` carry currency, and the language scan (adversarial test `brief_cross_domain_evidence_no_collapse`) proves no "the correct solution is…"/"OpenCode should implement…"/"you must execute" text appears. Ambiguity is surfaced (`AMBIGUOUS_TARGET` with bounded candidates), never resolved by guessing.

## 5. Evidence / Provenance Audit

**PASS.** Authority flows verbatim: `user_confirmed`/`ai_inferred`/`observed` strings from records and identity decisions are never upgraded or flattened — accepted learning is pinned `ai_inferred` (unit + MCP tests), rejected learning never enters `learning` (unit + MCP tests), preferences never become constraints (`brief_surfaces_confirmed_constraints_not_preferences`). Identity-sourced constraints honestly carry `source: "project_identity"` with `authority: None` rather than an invented rank. Mixed-authority inputs (USER_CONFIRMED constraint + AI_INFERRED learning + PROJECT_DERIVED facts on the same area) preserve all three distinctions in one brief (adversarial `brief_authority_conflict_is_preserved_not_resolved`).

## 6. Authority Conflict Audit

**PASS.** Conflicting decisions (≥2 shared significant tokens, differing currency) surface as `decision_conflicts` with both ids — never resolved by guessing (MCP test `brief_conflicting_decisions_surface`). AI inference cannot override confirmation: learning enters only as `ai_inferred` evidence with confidence, and constraints section only treats `user_confirmed` constraint records as `hard`. Scope (global/project/task) resolution is inherited from the fingerprint resolver, unchanged.

## 7. Scope / Isolation Audit

**PASS (with fix F2).** Workspace isolation: recall is workspace-scoped (`RecallScope::Project`/`Task` with canonical keys; global is never used by briefs), learning lists are workspace-or-global-or-task rows (store-level), skills likewise, tasks via `load_task_for` workspace gate. Cross-workspace task ids read as `TASK_NOT_FOUND` with no existence/content leak (unit + MCP + RPC + adversarial E2E). Task-scoped records do not leak into other tasks' briefs (adversarial `brief_task_scoped_record_isolation`). No project information crosses workspaces (single state dir, two servers, distinct repos — adversarial `brief_repository_identity_stable_and_distinct`). The two defects found in this area were secret-propagation, not scope-propagation (§31 below).

## 8. Taskless / Ambiguity Audit

**PASS.** A brief without `task_id` never selects a task: `task_state` is `None`, `scope.task_id` is `null`, and repository intelligence still works (adversarial `brief_taskless_never_invents_a_task` — with tempting tasks present). Ambiguous targets (same symbol name in two modules) yield `discovery: "ambiguous"` + bounded candidates + `AMBIGUOUS_TARGET` unknown + skipped traversal (adversarial `brief_ambiguous_symbol_skips_traversal`; unit + MCP tests for keyword ambiguity). No hidden target selection exists anywhere in `discover_targets`.

## 9. Unknown / Uncertainty Audit

**PASS.** The unknown taxonomy is complete and honest: `EMPTY_REPOSITORY`, `MISSING_IDENTITY`, `STALE_INDEX`/`FAILED_INDEX`/`UNKNOWN_FRESHNESS`, `NO_TARGET`/`AMBIGUOUS_TARGET`/`TARGET_NOT_FOUND`, `NO_RELEVANT_TESTS` (explicitly "not a claim that no tests exist"), `NO_HISTORY` (unavailable vs. absent distinguished in the details), `UNSUPPORTED_LANGUAGE` (file-level-only languages), `TASK_NOT_FOUND`, `NO_LEARNING`/`NO_MEMORY`/`NO_SKILLS` vs `NO_APPLICABLE_SKILLS` (distinguished via an unfiltered count probe that exposes no other-workspace data). A missing fact never becomes confidence; a corrupt state.db degrades to unknowns, never a crash or fabricated content (adversarial `brief_corrupt_state_db_degrades_to_unknowns`).

## 10. Freshness Audit

**PASS.** Live (`fresh`/`stale`/`unknown`) + persisted (`READY`/`STALE`/`FAILED`/`UNKNOWN` + `indexed_at` + revision) both reported. Stale ⇒ `STALE_INDEX` unknown + note + `risks` entry labelling structural evidence as last-indexed state. The full cycle (index → brief → modify → stale brief without new symbol → reindex → changed impact) is pinned by the real-binary E2E, including the assertion that the stale brief does NOT contain the new symbol (stale data is never presented as current). Restart after stale/failed states is covered by the E2E's restart-determinism leg and the FAILED-preservation unit test.

## 11. Repository Identity Audit

**PASS.** `RepoIdentity::from_workspace` (P6, unchanged): canonical root + remote + HEAD → `project_id`; the brief reports it with `repository_type`, `git_remote`, `commit_sha`, `identity_loaded`. Identity is stable across restarts, distinct across repos (adversarial `brief_repository_identity_stable_and_distinct` — two repos, one state dir, restart leg). Non-git workspaces stay distinct via canonical root. The brief cannot combine repositories: every store read is keyed by one canonical workspace root.

## 12. P6 Integration Audit

**PASS.** No duplicate P6 storage, no second parser, no second index, no second graph: the brief consumes `FactStore` reads, `impact::analyze`, `health::analyze_health`, `compute_freshness`, `repo_indexes` rows, and the facts lexical search — all canonical P6 surfaces. No new schema (v7 unchanged); no new persistent table (§33).

## 13. Impact Audit

**PASS.** One bounded traversal per brief; `depth ≤ 2` (request depth clamped; `> 99` rejected as invalid params — adversarial `brief_depth_bounds_enforced`); `max_nodes: 1000`; direct/transitive capped at 10 + totals + `truncated` flag + `impact_truncated` risk signal. Cyclic graphs terminate deterministically (unit test). Huge fanout (300 symbols calling one target) stays bounded (adversarial `brief_massive_store_stays_bounded`). Ambiguous/missing targets skip traversal with unknowns. Risk is a signal (level + indicators + blast radius) that also feeds `risks` — never a conclusion or authorization. Minor drift: `depth: 0` is silently floored to 1 by the impact call although `BriefRequest::depth()` documents a `0..=2` clamp — the doc comment now states the floor explicitly (F3; a brief without traversal has no impact value, `impact_analyze` owns depth 0).

## 14. Health Audit

**PASS.** `analyze_health` (pure over store) then task-relevance filtering: `STALE_INDEX` always relevant; otherwise keyword/target-path overlap with the finding location; ≤ 10; deterministic order. Unrelated findings are excluded by construction (a probe over a repo with an unrelated ORPHAN in `src/two` while the task targets `src/one` shows only relevant findings or none). Health findings never become decisions — warning/error severities feed `risks` as signals only.

## 15. Test Relevance Audit

**PASS.** Tests come from the impact traversal (`TestFact.tested` linkage + module containment, the canonical P6 rules) then module containment over relevant files; sorted, deduped, capped at 10. "No relevant test" is explicitly distinguished from "no tests exist" in the unknown's detail text. Unsupported-parser discovery is explicit through `UNSUPPORTED_LANGUAGE`.

## 16. History / Recall Audit

**PASS.** One bounded `recall` query per brief (task scope when the brief names a task — task history is invisible otherwise), ≤ 8 excerpts carrying kind/session/stale/task-match provenance, no transcripts, no raw payloads, no row ids (session ids are canonical `sess::` values). Huge history (60 seeded events) stays bounded and deterministic (unit test). Superseded knowledge stays superseded (record lifecycle is P1-enforced upstream). A keyword-less request reports `NO_HISTORY` rather than dumping sessions. History excerpts are redacted at the P2 write seam (`redact_secrets_public` before storage — verified in `history.rs`).

## 17. Negative Knowledge Audit

**PASS.** Rejected learning surfaces only as `rejected_learning` negative entries (unit + MCP tests) — never as positive evidence or constraint. Failed tasks and failed validations feed `failed_task`/`failed_validation` negatives. Superseded decisions keep `current: false` and are never presented as current. Negative evidence is keyword-relevance-filtered like everything else.

## 18. Learning Audit

**PASS.** Only `Accepted` candidates enter `learning` (≤ 5, confidence desc + id tiebreak), always `authority: "ai_inferred"`. Rejected → negative knowledge only. Candidate/deferred/expired never surface (`NO_LEARNING` instead). Cross-workspace and cross-task learning visibility is store-level (workspace-or-global-or-exact-task rows). The brief performs no detection, evaluation, confirmation, or rejection.

## 19. Skills Audit

**PASS WITH FIXES (F1, F2 — §31).** Applicability information only: registry actives evaluated against repo languages/task keywords (`applicable` + reason; unscoped skills marked uncertain; non-matching filtered), task-referenced skills via P6 read-time resolution (resolved + opaque unresolved), sorted applicable-first, ≤ 8. No execution, approval, publication, or modification anywhere in the brief; the "must never order skill execution" language is pinned by the existing MCP test plus the adversarial no-decision-language scan. Missing/deprecated/rolled-back skills: `NO_SKILLS` vs `NO_APPLICABLE_SKILLS` distinguished; unresolved refs stay opaque.

## 20. Task Audit

**PASS.** `task_resume_snapshot` is read-only; the brief adds no checkpoint, transition, lease, heartbeat, history, or learning write — pinned by `brief_with_task_id_reads_task_state_read_only` (status/version unchanged after a brief), `brief_concurrent_requests_agree_and_write_nothing` (directory listing unchanged), and the workspace-write-nothing E2E. Failed tasks feed negative knowledge. Terminal-intent notes report terminal states as terminal.

## 21. Context Composer Audit

**PASS.** No second context engine: records arrive caller-resolved through the existing fingerprint pipeline (`context_record_excerpts` → `ContextRetriever::search` → `fingerprint::resolve_context`), and the brief only truncates/sorts/bounds them (max 8, sorted by id). Ranking, authority resolution, and scope resolution are all inherited. One asymmetry documented (F4): the handler's record-keyword enrichment includes task-description tokens while the assembler's scope-keyword enrichment includes checkpoint-summary tokens — the sets overlap but are not identical; records resolve against the broader set. No trust consequence (records carry their own authority/scope tags verbatim); documentation corrected.

## 22. Fingerprint Audit

**PASS.** Resolved records carry kind/authority/scope verbatim; preferences remain `PREFERENCE`-categorized records and never enter `constraints` (only `constraint`-kind records and identity constraints do). Style cannot become technical fact: the brief's sections are typed, not free-text buckets.

## 23. Intent Audit

**PASS.** Task-intent notes surface terminal intents as terminal ("referenced intent … is completed/cancelled/rejected") via the P5/P1 successor-status lookup; a completed intent never reads as active. Task-scoped intents require their task (store-level, P1).

## 24. Constraint Audit

**PASS.** Only identity constraints (declared, `hard`, redacted at the identity write seam after fix F2b) and `user_confirmed` constraint-kind records are `hard`; other constraint records are `observed`. An AI inference never becomes a hard constraint; a preference never becomes a constraint at all (test-pinned).

## 25. Decision Audit

**PASS.** Current decisions appear with `current: true` (accepted/proposed); superseded/deprecated keep `current: false`; conflicts surface with both ids; authority and scope preserved; P7 never invents a resolving decision (test-pinned).

## 26. Ranking Audit

**PASS.** No new ranking: section-internal order reuses source ranking (facts score order with its deterministic kind/name/path tiebreaks, memory resolver order, recall order, fingerprint resolution order); brief-level combination uses stable sorts on (key/name/id/confidence+id) and `BTreeMap`/`BTreeSet` accumulation. The only `HashMap`s in the path are membership/adjacency maps with sorted emission (P6-verified). Reordered keyword inputs produce identical briefs (unit + adversarial `brief_keyword_order_determinism`).

## 27. Determinism Audit

**PASS.** Same repository + task + context + database state ⇒ byte-identical briefs: repeat-assembly equality, reordered-keyword equality, concurrent-agreement (8-way), and restart byte-equality over RPC (real-binary E2E leg 5) are all test-pinned. `now` enters only through documented stateful fields (lease-staleness, freshness) whose changes are state changes, not nondeterminism. FTS row order never leaks (recall applies the deterministic rank; the brief sorts its outputs).

## 28. Bounding Audit

**PASS.** Every section has an explicit cap (keywords 16, files 10, symbols 10, dependencies 10, impact 10+10, tests 10, health 10, history 8, memory 5, learning 5, negative 5, skills 8, constraints 10, decisions 8, conflicts 5, risks 8, records 8, ambiguity 5; excerpts 240/500 chars; depth ≤ 2; `max_nodes` 1000), plus the global 256 KiB `bounded_response` envelope with structural windowing. A 300-symbol / 30-module store yields a bounded brief with direct edges capped at 10 (adversarial `brief_massive_store_stays_bounded`). `list_skills`/`list_candidates`/`recall` limits are clamped server-side. No unbounded `Vec` growth or SQL result set exists on the brief path.

## 29. MCP Input Audit

**PASS.** Empty scope is `invalid_params` (never a dump); blank targets, `..` traversal, and NUL are rejected; `depth > 99` rejected, mid values clamp to 2; conflicting target parameters follow a documented deterministic precedence (symbol → path → module in discovery; the same for impact targeting); oversized task text is excerpted (500 chars); `workspace_root` canonicalizes through the registry (forged roots resolve to the caller's own namespace). No cross-workspace access; no accidental mutation (read-only handler, no lock taken).

## 30. MCP Output Audit

**PASS.** No SQLite row ids, no raw event payloads, no `payload_json`/`dedup_key`/`digest` internals (adversarial `brief_output_hygiene_no_internal_fields`); session ids are canonical `sess::` values; task ids are opaque `task::` values; secrets are redacted (after fixes) at every new seam; no filesystem paths outside the workspace; skill artifact *contents* never appear (metadata + applicability only). The 25-tool surface stays free of CRUD getters (re-verified live `tools/list`).

## 31. Security / Redaction Audit

**PASS WITH FIXES — 3 defects found and fixed (F1, F2, F2b).**

**F1 (HIGH)** — *Secret in skill description reaches the Engineering Brief.* `skill propose` stored `description`/`purpose` verbatim (the one unredacted free-text write seam), `validate_skill_content` secret-scans only the SKILL.md content, and an approved active skill's description flows into `brief.skills[].description`. Reproduced end-to-end through the real binary: history → accepted learning (confidence 0.74) → `skill propose` (learning-backed, poisoned description `api_key=sk-…`) → `validate` (passes — content clean) → `approve` (publishes) → brief surfaces the secret verbatim. **Fixed at the write seam** (redact `description`+`purpose` in both propose branches, matching remember/task/record_memory policy) **plus defense-in-depth** in the brief projection. Regression: `brief_secret_in_skill_description_never_reaches_brief`.

**F2 (MEDIUM)** — *Task `skill_refs` stored unredacted.* `create_task`/`set_task_skill_refs` validate shape but pass refs verbatim; the brief's unresolved-ref projection echoes them into `skills[].name`. A secret-styled ref would persist and surface. **Fixed at both write seams + brief projection defense.** Regression: `brief_secret_in_task_skill_ref_not_echoed` (asserts the ref still surfaces — redacted, not dropped).

**F2b (MEDIUM)** — *Secret in identity free-text reaches the brief as a hard constraint.* `update_identity` redacted nothing: constraints, decision titles/descriptions, architecture summaries, roadmap items, description, repository_url, sprint, milestones — all persisted verbatim to `.codebro/project_identity.json`. The brief surfaced identity constraints verbatim (labelled `hard`!) and decision titles. Reproduced through the real binary (`add_constraints: ["deploy only with token sk-… present"]` → verbatim hard constraint in the brief). **Fixed at the write seam** (every `update_identity` free-text field flows through `redact_secrets_public`, including the `push_unique_strings` lists and decision/roadmap constructors) **plus defense-in-depth** re-redaction in the brief's constraint, decision-title, architecture-summary, memory-value, history-excerpt, and learning-proposition projections (covers legacy persisted rows that predate the write-seam fix). Regression: `brief_secret_in_identity_constraint_not_surfaced` (asserts the constraint still surfaces — redacted, not dropped).

All other seams re-verified: task titles/descriptions (redacted, probe B), history (write-time redaction + brief projection re-redaction), memory values (`record_memory` redacts + projection), identity JSON (P6 `bound_identity_json` fix holds; identity free-text now redacted at update_identity), filenames/paths (canonical repository structure). INFO note: the brief echoes the caller-supplied ad-hoc `task` text excerpted but unredacted — same-channel caller echo, never persisted, consistent with the P0–P6 convention (redaction at persistence seams; the caller already holds the text).

## 32. Concurrency Audit

**PASS.** The handler takes no mutation lock and performs only short read transactions; SQLite is WAL. Briefs run concurrently with `reindex`, task mutation, and skill mutation without deadlock (test-pinned; briefs racing a reindex may legitimately differ in freshness — each must merely be well-formed, which is asserted). Concurrent briefs on stable state agree byte-for-byte. No mixed-version-read hazard exists beyond the documented atomic-facts.json-swap semantics (old-or-new, never torn).

## 33. Cross-Process Audit

**PASS (documented boundary).** The restart legs (real-binary E2E, identity test) pin persisted-state determinism across processes. The pre-existing single-writer assumption for cross-process mutations is unchanged and documented; P7 claims no distributed consistency and adds none.

## 34. Persistence Audit

**PASS.** P7 persists nothing new — verified: schema stays v7, no new tables, no `repo_indexes`/history/record writes on any brief path (grep + write-nothing tests). This is intentional and documented: the brief is a pure read/composition layer, so no stale "brief storage" can exist.

## 35. Failure / Recovery Audit

**PASS.** Every source failure degrades to an explicit unknown, never to silence or false certainty: corrupt state.db → brief still answers with unknowns (adversarial); history/FTS failure → `NO_HISTORY` ("unavailable, not absent"); learning/skill read failures → empty + unknowns; missing identity → `MISSING_IDENTITY` with defaults; empty store → `EMPTY_REPOSITORY`; failed reindex → `FAILED_INDEX` + preserved last-good metadata (adversarial `brief_failed_reindex_marks_failed_index` asserts `persisted_status: "FAILED"` and the `FAILED_INDEX` unknown on a corrupted tree).

## 36. Partial Evidence Audit

**PASS.** Partial availability never looks fully authoritative: a stale index with available history still carries `STALE_INDEX` + the staleness note + the `stale_index` risk; a healthy P6 index with unavailable history carries `NO_HISTORY`. Unknowns are additive — each degraded source adds its own explicit marker rather than shrinking the unknowns list.

## 37. Cross-Domain Consistency Audit

**PASS.** Composite probe: constraint ("never rewrite alpha without review") + accepted learning + impact on the target + task evidence — all four domains appear with their own categories and authorities, no collapse into "modify X instead", no decision-like language anywhere in the output (adversarial `brief_cross_domain_evidence_no_collapse` scans for the forbidden phrases). The brief preserves contradictions for OpenCode to reason about.

## 38. Usefulness Audit

**PASS.** Realistic flows verified end-to-end through the real binary: central-module bug fix (explicit symbol target → impact + direct callers + risk), leaf-module feature work (keyword discovery → relevant files/symbols), high-fanout refactor (300-caller store → bounded direct edges + truncated signal + blast radius), failing-test investigation (impact-linked tests + `NO_RELEVANT_TESTS` honesty), dependency-cycle investigation (cyclic-graph termination), task lifecycle continuation (task state + checkpoint + intent note + skill refs), stale-index work (freshness + staleness labelling). In each case the brief carries the evidence OpenCode needs to reason; the coding decisions remain OpenCode's.

## 39. P6 Debt Audit

**PASS — not worsened.** Task↔skill association remains the P6 read-time resolution (workspace-scoped, exact-match, bounded by `MAX_TASK_SKILL_REFS`, deterministic sorts — re-verified this audit). `task_id` namespace mixing remains per-seam. Lifecycle vocabulary sprawl unchanged. `engineering_memory.json` vs SQLite separation respected (brief reads the JSON store through the canonical runtime; SQLite keeps counts/status only). Documentation layout: the two P7 drift points found were fixed in place. New debt: none.

## 40. Test Architecture Audit

Pre-existing P7 tests are genuinely behavioral (lifecycle, isolation, determinism, bounds, concurrency, real-binary E2E through stdio RPC — no happy-path-only coverage found for the claims that matter). This audit added **16 adversarial regression tests** (`crates/mcp-server/tests/p7_brief_adversarial.rs`) covering the attacks above, all through the real binary, all hermetic (tempdir + `CODEBRO_STATE_DIR`; `CODEBRO_SKILLS_DIR` isolation wherever skill publication is exercised — a harness requirement this audit identified after the first probe accidentally published into the container's home skills dir; the existing suite was verified to already isolate or never publish).

## 41. Findings

| ID | Sev | Location | Finding |
|----|-----|----------|---------|
| F1 | HIGH | `mcp/mod.rs` skill propose + `engineering_brief.rs` skills projection | Skill descriptions/purposes persisted unredacted (the one unredacted free-text write seam) and surfaced verbatim in brief `skills[].description`. Reproduced through the real binary via the learning-backed publish path. **Fixed**: redact at propose (both branches) + brief projection defense-in-depth. Regression: `brief_secret_in_skill_description_never_reaches_brief`. |
| F2 | MEDIUM | `context-runtime/tasks.rs` create_task/set_task_skill_refs + brief task_ref projection | `skill_refs` persisted unredacted; brief echoed unresolved refs verbatim in `skills[].name`. **Fixed**: redact at both write seams + brief projection (resolved-name fallback and unresolved echo). Regression: `brief_secret_in_task_skill_ref_not_echoed`. |
| F2b | MEDIUM | `mcp/mod.rs` update_identity + `engineering_brief.rs` constraints/decisions/architecture projections | Identity free-text (constraints, decision titles/descriptions, architecture summaries, roadmap items, and every scalar setter) persisted verbatim — another unredacted write seam — and the brief surfaced identity constraints (as `hard`!) and decision titles verbatim. Reproduced through the real binary. **Fixed**: redact every `update_identity` free-text field at the write seam + defense-in-depth re-redaction in the brief's constraint, decision, architecture, memory, history, and learning projections (covers legacy rows). Regression: `brief_secret_in_identity_constraint_not_surfaced`. |
| F3 | LOW | `engineering_brief.rs::BriefRequest::depth` | Doc claimed `0..=2` clamp; the impact call silently floors 0 → 1. **Fixed**: doc comment states the floor (behavior kept — a brief without traversal has no impact value). |
| F4 | LOW | `mcp/mod.rs` engineering_brief handler + `P7_IMPLEMENTATION.md` §5 | Doc claimed the assembler "re-derives the same enrichment" as the handler's record keywords — false (handler adds task-description tokens; assembler adds checkpoint-summary tokens). **Fixed**: comment + doc describe the sibling enrichments honestly. |
| I1 | INFO | `credentials/mod.rs` test | Pre-existing chmod-based failure test fails when the suite runs as root (root ignores 0o555). Container artifact; passes as non-root. Not P7, not changed. |
| I2 | INFO | `engineering_brief.rs` task_ref skills | Resolved task-ref skills report `version: 0` (P6 read-time resolution carries no version). Inherited P6 design; cosmetic. |
| I3 | INFO | `engineering_brief.rs` task echo | The ad-hoc `task` string is echoed excerpted but unredacted — same-channel caller echo, never persisted, consistent with the codebase convention (redact at persistence seams). No action. |

Blocking: F1 was blocking (HIGH, security); fixed and pinned. F2 and F2b (MEDIUM, security) fixed and pinned. F3/F4 documentation corrections. I1–I3 accepted.

## 42. Fixes Applied

5 fixes (F1, F2, F2b, F3, F4), 16 permanent regression tests (one adversarial suite file), 0 new tools, 0 schema changes, 0 architecture changes. Files touched: `mcp-server/src/mcp/mod.rs` (skill-propose + update_identity redaction, enrichment comment), `mcp-server/src/engineering_brief.rs` (eight projection redactions + depth doc), `context-runtime/src/tasks.rs` (skill_refs redaction at both write seams), `mcp-server/tests/p7_brief_adversarial.rs` (new, 16 tests), `docs/evolution/P7_IMPLEMENTATION.md` (enrichment drift), plus this audit document, `docs/MCP_API_V1.md`, `AGENTS.md`, and `CHANGELOG.md` (redaction-guarantee wording). All edits are additive-or-bugfix; no P0–P6 semantic change (the two write-seam redactions close gaps, they do not alter any legitimate stored value).

## 43. Remaining Debt

1. Depth-0 briefs are floored to 1 (documented; `impact_analyze` owns depth 0).
2. Record-keyword enrichment (handler) and scope-keyword enrichment (assembler) are sibling, not identical, sets (documented; no trust consequence).
3. `repo_indexes` written only by MCP `reindex` (P6 carried limitation).
4. Task-ref resolved skills carry no version (P6 read-time resolution design).
5. P8 explicitly not started.

## 44. Final Verdict

**P7 VERIFIED WITH NON-BLOCKING DEBT.** No CRITICAL findings. The one HIGH and two MEDIUM findings were reproduced against the live implementation through the real binary, fixed at their write seams with projection defense-in-depth, and pinned by permanent adversarial regression tests. Architecture boundary (no decisions, no execution, no model calls, no background work), evidence/provenance, authority, scope/isolation, freshness, impact, health, history, learning, skills, tasks, security/redaction, determinism, boundedness, concurrency, MCP surface, persistence-free composition, failure recovery, partial-evidence honesty, cross-domain consistency, P0–P6 regression, and real-binary E2E all pass. Residual items are documented limitations, not trust defects.
