# CodeBro P6 Post-Implementation Audit — Engineering Intelligence Layer

**Verdict: P6 VERIFIED WITH NON-BLOCKING DEBT**
**Date:** 2026-09-07 · **Auditor:** adversarial post-implementation review (implementation report assumed hostile)
**Scope:** all P6 files + every P6 touchpoint in P0–P5. P0–P5 frozen; P7 explicitly not started.
**Method:** read the code first, wrote failing probes against the live implementation, fixed confirmed defects with regression tests, re-ran everything.

## 1. Executive Summary

P6 is real, correctly architected, and — after this audit's fixes — trustworthy. The layered design (repository → discovery → parse → normalize → facts.json → FactStore/impact BFS → risk/health/freshness → 24 semantic MCP tools) holds up under adversarial testing: no agent loop, no scheduler/daemon/watcher, no model calls, no skill/task execution, no cross-workspace leakage, deterministic bounded outputs everywhere I probed.

The audit broke P6 seven times. One HIGH (secret-value leakage through a broken hand-rolled redactor), three MEDIUM (false-Fresh on untracked edits, self-inflicted Stale from `.codebro/` output, silent 100-entry diff truncation, last-good metadata zeroed on failed reindex — four symptoms, grouped as three findings), three LOW (lexical-canonical fallback losing the root, `git remote -v` abort on blank lines, dead `RiskInput::is_generated`). All seven are fixed with eight permanent regression tests. Remaining items are INFO-level documented limitations, not trust defects.

Full suite: **1322 passed / 0 failed** (1314 pre-audit + 8 new). Clippy `-D warnings`: clean. `cargo fmt --check`: clean. Real-binary E2E: pass. 24 tools: confirmed. Schema v7. `~/.codebro` untouched by the suite (home `state.db` mtime 00:26, suite ran ~21:30).

## 2. Audit Scope

Read in full: `core/repo_state.rs`, `indexer/init/engineering.rs`, `parsers/file_classify.rs`, `parsers/languages.rs`, `impact/{mod,risk,health}.rs`, `context-runtime/{db,repo_index,history,learning,tasks,workspace}.rs`, `mcp-server/{mcp/mod,mcp/facts,doctor,engineering_context,history_capture,lib}.rs`, `mcp-server/tests/p6_engineering_e2e.rs`, plus `P6_IMPLEMENTATION.md`, `PHASE4_PLAN.md` (via evolution docs), `MCP_API_V1.md`, `AGENTS.md`, `CHANGELOG.md`, `P5_POST_IMPLEMENTATION_AUDIT.md`. Compared every doc claim against code; every claim below cites a file.

## 3. Architecture Verification

**PASS.** P6 adds index/parse/measure/inform capability only. Verified by grep over all P6 files: zero `Command::spawn`/threads/timers/cron/sleep/watchers (the only `Command::new("git")` calls are synchronous request-driven VCS probes in `core/repo_state.rs`, a pre-existing pattern), zero LLM/model/provider references (one doc-comment mention of "Never LLM-generated" as a constraint, not a call). `reindex` reuses the existing per-workspace mutation lock and `spawn_blocking` for the synchronous init pipeline (responsiveness, not background work — no loop, no queue). No skill execution, no task transitions, no second source of truth (SQLite holds counts/status only; facts.json holds symbols/edges). Dependency direction script: OK.

## 4. Repository Identity Audit

**PASS WITH FIXES (F5, F6).** Identity = canonical root + remote + HEAD → `project_id` SHA-256[..16]; `same_workspace()` compares canonical roots only. Stable across restarts/calls, distinct per workspace, symlinked aliases collapse (store key and identity agree). Two defects found and fixed (see §31): lexical fallback dropped the leading `/` for missing paths with `..` above root (returned `"c"`); `git_remote_of` aborted the whole scan on a blank line (`?` inside the line loop). `project_id` legitimately changes when the git remote changes (remote is part of the hash) — documented behavior, and harmless because `repo_indexes` is keyed by workspace root, not `project_id`. Non-git workspaces stay distinct via canonical root alone. No collision found: distinct roots ⇒ distinct ids (remote only narrows, root always feeds the hash).

## 5. File Index Audit

**PASS.** `FileRecord` ids are path-derived (`file::<rel>`), hashes SHA-256 over caller-gated reads, classes deterministic-sorted, contents never stored. Unsupported languages (C/C++/shell/TOML/YAML/JSON/markdown) preserve file-level intelligence with an explicit `parser_limitation` and contribute zero symbols (covered by e2e `c_file_has_file_level_intelligence_but_no_invented_symbols`). Rename/delete rebuild from the live file list — no orphans (e2e asserts every `Calls` edge resolves post-deletion). Symlink safety: discovery uses `follow_links(false)`; symlinked files never enter digests. `scan_current_digests` skips oversized/unreadable deterministically. No traversal outside the workspace (all paths workspace-relative).

## 6. Incremental Index Audit

**PASS WITH FIX (F2).** `diff_digests` kernel is pure/sorted/deterministic. Pre-audit defect: the MCP layer carried a *second* implementation (`diff_digests_for_mcp`) that silently truncated lists at 100 with no signal — the P6 doc's "one shared kernel" claim was false. Fixed: MCP delegates to the kernel and reports exact totals + `truncated` (deterministic head-truncation). Unchanged files keep byte-identical symbol IDs across reindex (e2e). Parse-cache reuse is pre-existing init behavior, correctly surfaced as `needs_reparse()`.

## 7. Symbol Audit

**PASS.** No new extractor; tree-sitter remains the only symbol source (Rust/Python/JS/TS/Go). Deterministic IDs across reindex (e2e). No invented symbols for unsupported languages. Caller attribution, visibility, and test linkage are pre-existing pipeline behavior, unchanged by P6. Parser limitations are disclosed via `supported_languages` + per-file `parser_limitation` — no over-claimed precision found.

## 8. Graph Audit

**PASS.** No graph DB; FactStore + impact BFS is the graph (as documented). Provenance preserved (verified 0.95 / heuristic 0.55 decayed 0.85^hops). Deletion rebuilds the model — no orphaned edges (probed + e2e). Cycles cannot hang traversal: BFS is visited-set bounded with `max_nodes` ceiling and depth ≤ 5 enforced by `validate_opts` (depth 99 rejected, tested). Deterministic ordering verified by repeat-query equality (unit + e2e).

## 9. Graph Depth / Boundary Audit

**PASS.** Depth 0/1/2/5 exercised (e2e); >5 rejected as invalid params; `max_results`/`max_nodes` bound output with explicit `truncated` + `truncation_reason` metadata. `max_nodes` has no upper clamp, but traversal is inherently store-bounded (each node visited once) — no explosion vector. Adjacency uses `HashMap` internally but all emitted orderings are sorted; determinism tests pass.

## 10. Impact Audit

**PASS WITH FIX (F7).** Targets file/symbol/module/package resolve correctly; `NotFound` returns a stable LOW-risk shape instead of an error. Direct/transitive separation, references, tests, modules, packages, evidence, and traversal metadata all present and bounded. Risk is a deterministic signal (HIGH/MEDIUM/LOW, ≤8 sorted indicators, templated blast radius) — never an authorization. Removed dead `RiskInput::is_generated` (set by the caller, never read by `assess_risk` — false impression of generation-awareness). Residual heuristic substring matching (`store`, `core`, …) is documented triage aid, INFO only.

## 11. Health Audit

**PASS.** `analyze_health` is pure over store + staleness flag: CYCLE (bounded DFS, ≤8, deterministic), HIGH_FANOUT/FANIN (≥20), ORPHAN (docs/config excluded), UNRESOLVED_REFERENCE (validation `broken_index` count), STALE_INDEX (explicit flag only — never inferred), MISSING_TEST_ASSOCIATION, LARGE_MODULE (≥100). Deterministic ordering (type → location → evidence), bounded (default 50, cap 500). Severity drives doctor outcome (Error→fail, Warning→warn, Info-only→pass); absent facts.json skips honestly. `BOUNDARY_CROSSING` exists as vocabulary but is never emitted — documented as deferred (INFO, not a false claim since nothing advertises it as active).

## 12. Index Lifecycle Audit

**PASS (INFO).** The six-state vocabulary exists, but only READY/FAILED are ever persisted, STALE is computed at read time, and DISCOVERING/INDEXING are never written. There is no transition validation — correctly so: a single writer (`reindex`) sets terminal status per run; there are no concurrent lifecycle actors to guard against. A failed run can never read as READY (absent rows read UNKNOWN; failure path now preserves last-good + FAILED). Crash mid-index leaves the previous facts.json (atomic write) and previous SQLite row — recovery is "reindex again", deterministic. No partial-index-masquerading-complete path found.

## 13. Persistence Audit

**PASS.** SQLite v7 `repo_indexes` holds derived metadata only (identity JSON ≤4 KiB, status, timestamps, revision, counts). No file contents/symbols/edges in SQLite — verified in `repo_index.rs` and migration comments. FTS is never consulted for index state; canonical data path is facts.json → FactStore. `get_repo_index` fabricates nothing (absent → in-memory UNKNOWN).

## 14. Migration Audit

**PASS.** v7 step is `IF NOT EXISTS` table + index, no backfill, crash-safe resume, idempotent re-run — all covered by existing tests (fresh v7, v1→v7 chain, v6→v7 preserving tasks, interrupted resume, idempotence). Full workspace suite green, so P0–P5 rows (records, events, sessions, candidates, skills, versions, tasks, checkpoints) survive. No new migration risk introduced by this audit's fixes (no schema change).

## 15. Workspace Isolation Audit

**PASS.** Store layer keys every read/write by `canonical_workspace_key` (lexical + symlink-resolving; probed A/B). `load_task_for` refuses cross-workspace task ids ("belongs to another workspace"). `resolve_task_skill_refs` matches skills within workspace-or-global only. MCP `reindex`/`repository_health` resolve through per-workspace `WorkspaceState` with isolated fact stores (existing `reindex_is_workspace_isolated` + `workspace_scoping` tests green). No caller-side-only filtering found on any P6 path. Forged workspace args canonicalize to the attacker's own namespace, never the victim's.

## 16. Task Isolation Audit

**PASS.** Task-scoped history requires its task; `resolve_task_skill_refs` loads the task through the workspace gate first (cross-workspace `task_id` reuse refused — covered by existing test asserting `/repo-b` resolution of a `/repo-a` task errors). Task-less queries cannot see task-scoped rows (exact `task_id` match semantics, unchanged from P5). No P6 path dereferences `skill_refs` into writes.

## 17. Context Integration Audit

**PASS.** `engineering_context::compose` unchanged by P6 and already disciplined: orientation + counts, keyword-ranked facts (capped, deduped), decisions, memory excerpts, evidence-journal status, freshness note, impact *names only* (no traversal), validation pointers — each provenance-tagged. No repository/graph/history dump path exists. USER_CONFIRMED authority still outranks derived engineering signals (resolution order untouched).

## 18. History Audit

**PASS.** Only `reindex` writes P6 history (`index_completed` with file/symbol/edge + diff summary; `index_failed` with bounded message). `impact_analyze` and `repository_health` are read-only — no recursion, no spam. `ImpactAnalyzed`/`HealthAnalyzed`/`RepositoryDiscovered` kinds exist but are never emitted (documented deferred opt-in; learning ignores them regardless). Append-only, workspace/task-scoped, redacted at the store seam.

## 19. Learning Boundary Audit

**PASS.** `kind_group` maps all five P6 kinds to `Ignored` (weight 0.0) — verified in code and covered by the "index completed → hypothesis is the canonical anti-example" comment. Repeated reindex/health/impact cannot form candidates. Task completions/validation failures still feed P3 through pre-existing rules. No pollution vector found.

## 20. Skills Boundary Audit

**PASS.** P6 touches no skill table, publishes/approves/modifies/executes nothing, writes no filesystem path outside `.codebro/facts.json` + its own SQLite row. `resolve_task_skill_refs` is read-time, workspace-gated, exact-match (id, else name), unmatched refs returned opaque, sorted, bounded (≤16 by validation). Applicability facts (languages, workspace shape) flow through existing read paths and confer no execution authority.

## 21. Task Boundary Audit

**PASS.** No scheduler/queue/daemon/executor introduced. `reindex` never transitions tasks; it only holds the workspace mutation lock (shared fairly with task mutations — serialization preserved, no new global locks). Task lifecycle, leases, fencing, and optimistic concurrency untouched (P5 suite green).

## 22. Concurrency Audit

**PASS.** Mutating `reindex` holds the per-workspace lock across the blocking init run; reads take no lock and observe atomic facts.json swaps (old-or-new, never torn — `write_atomic` + fsync + rename). SQLite WAL + 5s busy timeout + single-writer assumption unchanged and still documented. No deadlock/partial-persistence path found; failure branch upserts FAILED + history in best-effort order that cannot corrupt (store failure never fails the tool response).

## 23. Cross-Process Audit

**PASS (documented boundary).** P6 claims no new guarantees: two processes indexing simultaneously last-writer-wins on facts.json (atomic each) with one FAILED/READY row each — no corruption, no phantom READY, but no distributed exclusion either. The single-writer assumption remains stated in code (`mutation_lock` scope comments) and docs. No invented locking added by this audit, per instructions.

## 24. Filesystem / Symlink Security Audit

**PASS.** Discovery: `follow_links(false)`, symlink files/dirs never enter digests. `canonical_root_of` + `canonical_workspace_key` agree (fixed to share semantics). No `..` escape (lexical normalization + canonical comparison). Health/context/impact perform no writes. `reindex` writes only `.codebro/facts.json` (atomic) + its SQLite row. No trusted-skill-directory write path reachable from P6.

## 25. Redaction Audit

**PASS WITH FIX (F1).** Central authority `redact_secrets_public` covers key=value secrets, bearer/PAT shapes, and URL credentials. Pre-audit, `repo_index::bound_identity_json` used a hand-rolled loop that renamed key *names* while persisting secret *values* (proven by probe output showing `"git_remote":"https://user:s3cr3t-pw@…"` and `"token"`→`"<redacted>"` with the `ghp_…` value intact). Fixed: identity JSON now flows through the central redactor before the 4 KiB bound. History index summaries carry counts only. Filenames/paths are canonical repository structure, not secrets (consistent with P0–P5 policy).

## 26. MCP API Audit

**PASS.** Exactly 24 tools (live `tools/list` assertion in e2e, re-verified this audit). No CRUD leakage (name scan in e2e). P6 strengthened five tools additively (`workspace_context`, `engineering_facts` unchanged-shape, `impact_analyze`, `repository_health`, `reindex`) — no param breaks. Malformed inputs (bad kind, unknown target_type, depth 99, bad direction/relationship) return `invalid_params` with guidance. `engineering_facts` still rejects empty-unfiltered queries instead of dumping the store.

## 27. MCP Response Boundary

**PASS WITH FIX (F2).** Facts ≤50, findings ≤50 (cap 500), indicators ≤8, identity JSON ≤4 KiB, history summaries bounded, incremental lists 100/entry-kind — now with exact totals + `truncated` flag (previously silent). No unbounded serialization path found; truncation is deterministic (sorted head).

## 28. Determinism Audit

**PASS.** Identity, file IDs, diffs, edges, traversal, impact, health, risk, and context enrichment all sort before emitting (BTree maps/sets or explicit sorts at every seam). Repeat-run equality is test-pinned for symbols, impact, health, risk, and reindex. `HashMap` usages are membership/adjacency only with sorted emission. The `RepoState` byte-sort quirk is deterministic (same bytes → same hash); untracked-content hashing sorts names first. Filesystem enumeration order cannot leak (WalkDir output is hashed by path-keyed maps).

## 29. Failure / Recovery Audit

**PASS WITH FIX (F4).** Parser failure → file-level record, never invented symbols. Unreadable/oversized → deterministic skip. DB corruption → quarantine + recreate (P0 semantics, untouched). Interrupted indexing → previous atomic outputs stand; status never false-READY. Failed `reindex` now preserves last-good counts (previously zeroed) and records `index_failed`. Non-git staleness reports `unknown` — explicit, never a false claim. The system fails loudly at every seam I attacked.

## 30. Regression Audit

**PASS.** `cargo test --workspace`: 1322/1322 green (P0–P5 suites included: context, fingerprint, intent, remember/forget, recall, learning, skills, tasks, checkpoints, fencing, optimistic concurrency, redaction, isolation, migration, real-binary MCP). No P6 change altered P0–P5 semantics: the only shared-code edits are additive-or-bugfix in `RepoState::capture` (hash vocabulary — git workspaces go Stale exactly once, then heal on reindex; documented), `RiskInput` (removed dead field; risk outputs for real inputs unchanged — verified by untouched risk expectations in the green suite), and additive MCP response fields.

## 31. Test Quality Audit

Pre-existing P6 tests are behavioral (lifecycle, isolation, determinism, bounds, E2E through the real binary — strong). Gaps this audit closed with regression tests: value-level redaction (was key-name theater), untracked-content freshness, `.codebro/` self-staleness, canonical fallback, remote-parse robustness, diff-kernel delegation + truncation honesty, failure preservation. Scratch adversarial probes (`p6_audit_probes.rs`) confirmed all three runnable defects before the fix and were removed after folding permanent tests into `core/repo_state.rs` (5), `context-runtime/repo_index.rs` (1), `mcp-server/mcp/mod.rs` (2).

## 32. Documentation Audit

Drift found and fixed: P6 doc claimed a single shared diff kernel (false until fix C — now true); `incremental` shape undocumented totals (now in MCP_API_V1 + P6 doc); failure-preservation behavior (documented); redaction via "shared seam" (was false for repo_index — now true); non-git freshness honesty (added to MCP_API_V1 §P6 + P6 limitations); hash-vocabulary change with one-time-Stale upgrade note (P6 limitations + CHANGELOG). Tool count (24), schema (v7), languages, graph/health/impact semantics all match code. No gratuitous rewrites.

## 33. Architectural Debt Review

P0–P5 known debts: (1) task↔skill association — P6's read-time `resolve_task_skill_refs` + documentation resolves it without a join system: **resolved, correctly scoped**. (2) task_id namespace mixing — untouched, still enforced per-seam: **ignored correctly**. (3) lifecycle-model sprawl (IndexStatus vocabulary vs persisted subset) — **documented as INFO**; no enforcement theater added. (4) engineering_memory.json vs SQLite separation — P6 respected it (counts-only in SQLite): **not worsened**. (5) documentation layout — drift fixed in place: **improved**. New debt introduced: none (dead `ImpactAnalyzed`/`HealthAnalyzed`/`RepositoryDiscovered` kinds and library-only `scan_current_digests`/`collect_architecture` are declared extension points, not hidden dead weight — recorded as INFO).

## 34. Findings

| ID | Sev | Location | Finding |
|----|-----|----------|---------|
| F1 | HIGH | `context-runtime/src/repo_index.rs:bound_identity_json` | Custom redactor masked key names, leaked secret values + URL credentials to SQLite. **Fixed**: central `redact_secrets_public`. |
| F2 | MEDIUM | `mcp-server/src/mcp/mod.rs:diff_digests_for_mcp` | Duplicate diff implementation silently truncated at 100 entries. **Fixed**: delegates to `init::engineering::diff_digests`; totals + `truncated` flag. |
| F3 | MEDIUM | `core/src/repo_state.rs:capture` | Untracked content edits read `fresh` (names-only hash); un-ignored `.codebro/` output read `stale` immediately after index. **Fixed**: bounded content hashing + `':!.codebro'` pathspec. Residual: non-git → honest `unknown` (documented). |
| F4 | MEDIUM | `mcp-server/src/mcp/mod.rs` reindex failure branch | Failure zeroed counts/revision. **Fixed**: preserves last-good row, status → FAILED. |
| F5 | LOW | `core/src/repo_state.rs:canonical_root_of` | Lexical fallback returned relative `"c"` for absolute inputs with `..` above root. **Fixed**: shared semantics with storage keys. |
| F6 | LOW | `core/src/repo_state.rs:git_remote_of` | Blank/malformed `remote -v` line aborted the whole parse via `?`. **Fixed**: pure `parse_first_remote_url` with line-local skips. |
| F7 | LOW | `impact-engine/src/impact/{risk,mod}.rs` | `RiskInput::is_generated` set but never read. **Fixed**: field removed. |
| I1 | INFO | `context-runtime/src/history.rs` | `ImpactAnalyzed`/`HealthAnalyzed`/`RepositoryDiscovered` never emitted — declared deferred opt-in; learning ignores them. No action. |
| I2 | INFO | `indexer/src/init/engineering.rs` + `repo_index.rs` | No index-state transition validation — correct for a single-writer status field; STALE is read-time. No action. |
| I3 | INFO | `indexer/src/init/engineering.rs` | `scan_current_digests`/`collect_architecture` are library API without live callers; live freshness is hash-based. Documented; no action. |
| I4 | INFO | `impact-engine/src/impact/{risk,health}.rs` | Risk substring heuristics + alphabetically-ordered cycle locations are documented triage aids, not precision claims. No action. |
| I5 | INFO | `core/src/repo_state.rs:capture` | Byte-wise sort of tracked/diff blobs is semantically odd but deterministic and equality-safe. No action. |

Blocking (P6-gating): F1–F4 were blocking; all fixed and regression-pinned. F5–F7 non-blocking hardening, fixed. I1–I5 accepted tradeoffs.

## 35. Fixes Applied

7 fixes, 8 regression tests, 0 new tools, 0 schema changes, 0 architecture changes. Files touched: `core/repo_state.rs`, `context-runtime/repo_index.rs`, `impact-engine/impact/{risk,mod}.rs`, `mcp-server/mcp/mod.rs`, `docs/{MCP_API_V1.md,evolution/P6_IMPLEMENTATION.md}`, `CHANGELOG.md`. Full diff is additive-or-bugfix; no P0–P5 semantic change (one-time Stale on first post-upgrade freshness check for git workspaces, self-healing on reindex — noted in limitations and CHANGELOG).

## 36. Remaining Limitations

1. Non-git workspaces: freshness is honestly `unknown`; digest-driven staleness would need per-query tree walks (perf + new failure modes) — deferred, documented.
2. `file_digests` cover parsed sources + manifests; file-level-only languages rely on the git-state signal — pre-existing, documented.
3. Cycle detection bounded at 8 module-space cycles; huge graphs truncate deterministically.
4. `repo_indexes` written only by MCP `reindex` (bare CLI `init` reads UNKNOWN until next reindex) — by design, documented.
5. Risk/health are heuristic signals, never proofs or authorizations.

## 37. Final Verdict

**P6 VERIFIED WITH NON-BLOCKING DEBT.** No CRITICAL findings. The one HIGH and all MEDIUM findings were reproduced against the live implementation, fixed, and pinned with regression tests. Isolation (workspace + task), persistence, migration, determinism, concurrency, security, MCP bounds, P0–P5 regression, and real-binary E2E all pass. Residual items are documented limitations, not trust defects.
