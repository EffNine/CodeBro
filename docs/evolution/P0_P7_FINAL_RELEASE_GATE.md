# CodeBro P0–P7 Final Architecture + Release Gate

**Date:** 2026-09-08 · **Auditor:** independent final-gate review (entire P0–P7 system audited as ONE architecture, not phase-by-phase only)
**Scope:** all eleven crates, the 25-tool MCP surface, SQLite v7 persistence, all P0–P7 phase boundaries, plus fresh independent adversarial verification through the real binary (`crates/mcp-server/tests/final_gate_probe.rs`, 7 new real-binary tests).
**Method:** verify the mandated commands (fmt / clippy -D warnings / full workspace suite), audit dependency direction and phase boundaries from actual code, re-prove trust/isolation/redaction/determinism/persistence through an independently-written adversarial probe suite against the real `codebro` binary over stdio RPC, cross-check every major doc claim against the implementation, then classify findings.

---

## 1. Executive Summary

CodeBro P0–P7 is a coherent engineering infrastructure/runtime layer that can safely serve OpenCode and future AI coding agents. The north-star boundary — **OpenCode reasons, plans, codes, executes, and owns UX; CodeBro owns context, memory, history, learning, skills lifecycle, durable task state, repository intelligence, evidence, and deterministic retrieval** — holds structurally and behaviorally across every phase, verified from code and from live adversarial probes, not from documentation.

The gate found **zero CRITICAL and zero HIGH findings**. All known P7 audit defects (F1 HIGH, F2/F2b MEDIUM — secret-redaction write-seam gaps) were previously fixed and remain pinned by regression tests; this gate re-attacked every write seam independently (secrets through `remember`, `record_memory`, `update_identity`, `task` create/skill_refs, observation-minted evidence) and hunted them through every read path (`engineering_brief`, `context`, `recall`, `engineering_memory`, `task inspect`) — zero leaks. Cross-workspace zero-leakage, restart persistence, malformed-input rejection, trust-boundary enforcement, decision-neutrality, and concurrent brief+mutation were all re-proven with fresh probes.

Post-gate verification: **1383 passed / 0 failed** (1376 baseline + 7 new gate probes; the credentials chmod artifact passes as non-root uid 1000 — it only fails under a root container, pre-existing, not gate-relevant). Clippy `-D warnings` across all targets/features: clean. `cargo fmt --check`: clean. Schema v7 unchanged (correct — no persistence-correctness requirement emerged). 25 tools (live-verified). `~/.codebro` byte-identical before/after the full suite (state.db and memory.json md5-verified); real skill directories byte-identical; no repository mutation.

**Verdict: RELEASE READY WITH NON-BLOCKING DEBT.**

---

## 2. Scope

Everything P0–P7: `core`, `parsers`, `fact-store`, `identity-runtime`, `memory-runtime`, `sandbox-runtime`, `impact-engine`, `indexer`, `change-engine`, `context-runtime` (P0–P5 + P6 repo-index metadata), `mcp-server` (25 tools incl. P7 `engineering_brief`), SQLite schema v7, `.codebro/` JSON stores, docs (`AGENTS.md`, `MCP_API_V1.md`, `docs/evolution/*`, `CHANGELOG.md`). P8 explicitly out of scope and not started.

## 3. Architecture Overview

Layered runtime behind one MCP server over stdio:

```
Storage        .codebro/facts.json · engineering_memory.json · project_identity.json · state.db (SQLite v7 + FTS5)
Domain         context-runtime (records/events/sessions/learning/skills/tasks) · memory-runtime · identity-runtime · change-engine
Intelligence   fact-store · parsers (tree-sitter) · impact-engine · indexer
Composition    engineering_context (P0 packet) · engineering_brief (P7) · doctor · debugging (root-cause)
Interface      mcp-server (rmcp, 25 tools, per-workspace mutation lock)
```

Verified from code: no agent loop, no scheduler/daemon/watcher (all `tokio::spawn` are test-only; all `Command::new` in production are synchronous request-driven `git` probes for diff injection/doctor/identity), no LLM/model calls anywhere in P0–P7 (the only model consumer is the `consult` tool delegating to the user-configured provider — an explicit user-initiated request, not autonomous inference), no IDE surface.

## 4. Phase Responsibility Matrix

| Phase | Responsibility | Clear | Duplicated | Missing | Verdict |
|---|---|---|---|---|---|
| P0 | Context foundation (records/events/sessions/SQLite+FTS) | ✅ | none | none | PASS |
| P1 | Fingerprint/intent/authority/scope resolution | ✅ | none | none | PASS |
| P2 | Sessions/history/recall/passive capture | ✅ | none | none | PASS |
| P3 | Learning/inference (cautious, evidence-bound) | ✅ | none | none | PASS |
| P4 | Skills lifecycle (propose→…→deprecate; never executes) | ✅ | none | none | PASS |
| P5 | Durable task runtime (state, not execution) | ✅ | none | none | PASS |
| P6 | Repository intelligence (identity/index/impact/health/freshness) | ✅ | none | none | PASS |
| P7 | Engineering decision support (composition only) | ✅ | none | none | PASS |

No circular dependencies; later phases consume earlier ones only through their public runtime seams (`context_record_excerpts`, `ContextRetriever::search`, `resolve_task_skill_refs`, `task_resume_snapshot`, `FactStore` reads, `impact::analyze`, `analyze_health`, `compute_freshness`); no later phase reaches into earlier internals, and no earlier phase knows about later ones (P0's `context-runtime` has no dependency on brief/impact/indexer — confirmed by crate graph: `context-runtime` depends only on `core`).

## 5. Dependency Direction

Crate graph (from Cargo.tomls) is a strict DAG, enforced by `scripts/check_workspace_deps.sh` (re-run this gate: **OK**):

- `core` → nothing
- `parsers`, `fact-store`, `sandbox-runtime`, `change-engine`, `identity-runtime`, `context-runtime` → `core` only (+ memory-runtime → identity-runtime)
- `impact-engine` → core/fact-store/parsers
- `indexer` → core/fact-store/parsers/identity-runtime/impact-engine
- `mcp-server` → all of the above (top of the graph)

Conceptual flow matches the intended Storage → Domain → Intelligence → Composition → MCP. Storage never depends on MCP presentation; repository intelligence never depends on agent logic; the context engine never depends on Engineering Brief. No circular conceptual dependency exists.

## 6. Single Source of Truth

| Domain | Canonical store | Duplicates | Classification |
|---|---|---|---|
| Context records | `state.db` `context_records` | `context_records_fts` (derived search index) | INTENTIONAL DERIVED (transactional sync, §11) |
| Events/history | `state.db` `events` | `events_fts` (derived) | INTENTIONAL DERIVED (transactional sync) |
| Sessions | `state.db` `sessions` | `event_count` denormalized counter | INTENTIONAL DERIVED (same-tx update) |
| Learning candidates | `state.db` `learning_candidates` | accepted ⇒ AI_INFERRED record row (explicit promotion surface, supersede-linked) | INTENTIONAL DERIVED (auditable promotion, not silent copy) |
| Skills | `state.db` skills/version rows | published SKILL.md artifacts | INTENTIONAL DERIVED (atomic publication, content-hashed) |
| Tasks/checkpoints | `state.db` tasks/task_checkpoints | none | canonical |
| Repository index | `.codebro/facts.json` | `repo_indexes` row (counts/status/identity only — never contents/symbols/edges, schema-enforced) | INTENTIONAL DERIVED (last-good metadata) |
| Graph | FactStore relationships + impact BFS | none (no graph DB) | canonical |
| Health | pure function over store + staleness | none persisted | derived-at-read by design |
| Fingerprint | `state.db` records (fingerprint resolver) | none | canonical |
| Intent | `state.db` records (intent kind) | none | canonical |
| Engineering memory | `.codebro/engineering_memory.json` | SQLite holds no memory values | canonical (deliberate store separation) |
| Task↔skill association | task `skill_refs` (reference-only) + read-time `resolve_task_skill_refs` | no association table | canonical (documented debt, §42) |
| Repository identity | `RepoIdentity::from_workspace` (canonical root+remote+HEAD) | `repo_indexes.repository_identity` JSON (bounded, redacted) | INTENTIONAL DERIVED (cached projection) |
| Engineering Brief | nothing persisted (pure composition) | none | canonical-by-absence (verified: no writes on any brief path) |

No accidental duplication found. The one structural duplication — facts.json vs repo_indexes — is bounded, documented, and enforced (repo_indexes can never hold symbols/edges).

## 7. Trust / Authority

Six trust classes (`user_confirmed`, `ai_inferred`, `observed`, `project_derived`, `imported`, `system_derived`) with a total rank order (`authority_rank`: UserConfirmed 100 > ProjectDerived 50 > SystemDerived 40 > Imported 30 > Observed 20 > AiInferred 10). Verified from code and re-proven live:

- **AI_INFERRED → USER_CONFIRMED**: only via explicit supersede requiring `user_confirmed=true` at both MCP and store layers (`learning.rs:2033` refuses forged confirmation: "an AI inference can never promote itself"; gate probe `gate_ai_inference_cannot_self_confirm` re-proves it end-to-end, including the evidence-cited rejection message).
- **PROJECT_DERIVED → USER_CONFIRMED**: no path exists; identity constraints carry `authority: None` + `source: "project_identity"` in briefs — never a claimed user confirmation.
- **OBSERVED/PREFERENCE → constraint**: impossible by typed sections (only constraint-kind records and identity constraints enter `constraints`; preferences stay `PREFERENCE` records — test-pinned).
- **REJECTED → positive fact**: rejected learning only ever surfaces as negative knowledge (unit + MCP + adversarial tests).
- **SUPERSEDED → current decision**: superseded decisions keep `current: false` in briefs; resolution prefers the successor; the audit trail row is never deleted (soft status transition).
- Lifecycle floors (`lifecycle_for_authority`) keep authority and lifecycle coherent at the semantic write layer.

No phase silently upgrades authority. **PASS.**

## 8. Scope / Isolation

Hierarchy: global user → project → task; resolution order authority-rank first, then scope specificity (`scope_rank`). Verified live this gate (`gate_two_workspaces_zero_leakage`): two workspaces sharing one state dir, distinct repos — B's context/brief/recall contain none of A's records, task titles, memory, or project identity; A's own brief still surfaces A's data (sanity). Task-scoped records require their task at the store level; `load_task_for` refuses cross-workspace ids; `resolve_task_skill_refs` workspace-gates first. Forged `workspace_root` args canonicalize into the caller's own namespace (traversal probe: `../..` root leaked nothing from other workspaces). **PASS — zero leakage.**

## 9. Identifier Model

- Context records: `ctx::<hex>` opaque, store-minted. Sessions: `ses::<hex>`. Events: integer row ids (never exposed via MCP — excerpts carry canonical session ids only). Learning: `hyp::<hex>` (candidates) with record ids on acceptance. Skills: `skl::<hex>` + human names (name conflicts refused). Tasks: `task::<16hex>` minted existence-checked against the table (1000-attempt loop), worker: `wkr::<16hex>`, checkpoints: `cp::<16hex>`.
- All ids are opaque (no user-controlled content), prefixed by domain, and collision-checked at mint time. External task/skill ids are never accepted as input for creation (create mints; idempotency is keyed by `(workspace, idempotency_key)`).
- Known debt (unchanged, documented): `task_id` namespace mixing across seams — the MCP `task_id` field carries P5 canonical `task::` ids while P1 context-record `task_id` fields are caller-supplied opaque task identities; both are treated as opaque strings and never mixed in one query without a workspace gate. No concrete correctness or security failure observed (existence-check + workspace-keyed reads make collisions immaterial). Documented, deferred. **PASS with documented debt.**

## 10. Persistence

SQLite v7, WAL, `PRAGMA user_version` stepwise migrations v1→v7 (probe-first resume; idempotent re-run; interrupted-migration tests exist; full-suite green proves P0–P6 data survival). Transaction boundaries verified in code: record insert/supersede/remove sync FTS **inside** the caller's transaction; `record_event_in_tx` writes event + session heartbeat + FTS in one tx; checkpoint commit is row+pointer+event in one tx; skill publication is atomic file-swap (temp+fsync+rename) with DB rows committing in one transaction. Crash behavior: atomic facts.json swap (old-or-new, never torn); crash mid-index keeps previous facts.json + previous repo_indexes row; crash mid-transaction loses only the in-flight mutation (SQLite atomicity). Restart behavior re-proven live this gate (`gate_restart_preserves_state_and_brief_determinism`): running task survives a hard kill and reads `running` after restart (never auto-completed); identical brief requests after restart agree byte-for-byte. Corruption: quarantine-on-corruption with WAL-sidecar hygiene (P0); corrupt state.db degrades briefs to explicit unknowns (adversarial test). Lock handling: WAL + 5s busy timeout + single-writer assumption (documented; per-process mutation lock serializes in-process writers). **PASS.**

## 11. FTS5 / Derived State

FTS5 (`context_records_fts`, `events_fts`) is purely derived: every writer path (`insert_record`/`supersede_impl`/`remove_record`/`record_event_in_tx`/`sync_fts`/`sync_history_fts`) keeps FTS rows in lockstep inside the canonical row's transaction — there is no code path that writes a canonical row without its FTS sync or vice versa. Migration backfill for pre-v3 events exists and is bounded. Read paths never treat FTS as authoritative for state: record status/lifecycle always come from the canonical `context_records` row (FTS matches are candidacy only; `recall` filters by scope/status before ranking — FTS-bypass-proof), and index state comes from `repo_indexes`/facts.json, never FTS. Delete/rebuild covered by store tests (e.g. `DELETE FROM events_fts` rebuild test). If FTS ever went stale, canonical records remain correct and resolution still works (keyword recall degrades, authority resolution does not). **PASS.**

## 12. Redaction / Security Boundary

The single redaction authority is `redact_secrets_public` (`core/src/tools/shell.rs`) — regex patterns for sk-keys, bearers, api_key/password/token/secret assignments, ghp/glpat/xox tokens, URL-embedded credentials. Write-seam redaction verified at every persistence seam (grep-verified 16 call sites in the MCP layer + store seams in tasks/history/repo_index + engineering_brief projection defenses; `update_identity` free-text flows through `push_unique_strings` redaction). This gate independently re-attacked: secrets injected via `remember` (preference + observation), `record_memory`, `update_identity` (constraints/decisions/architecture/description), `task` (title/description/skill_refs) — then hunted through `engineering_brief`, `context`, `recall`, `engineering_memory`, `task inspect`: **zero leaks** (gate probe `gate_secrets_never_surface_through_any_read_path`). The P7 fixes (F1/F2/F2b) hold. Known accepted echo: the brief echoes the caller's own ad-hoc `task` text and `keywords` unredacted — same-channel caller echo, never persisted, convention-documented (the caller already holds the text). **PASS.**

## 13. MCP Surface Audit (25 tools)

| # | Tool | R/W | Lock | Purpose | Exposure |
|---|---|---|---|---|---|
| 1 | workspace_context | R | – | orientation | bounded |
| 2 | engineering_facts | R | – | ranked facts | bounded, clamped ≤50 |
| 3 | engineering_memory | R | – | memory resolution | budgeted |
| 4 | memory_stats | R | – | store stats | bounded |
| 5 | record_memory | W | ✅ | upsert memory | redacted |
| 6 | delete_memory | W | ✅ | delete (confirm-gated) | safe |
| 7 | update_identity | W | ✅ | identity update | redacted |
| 8 | apply_change | W | ✅ | guarded mutation | ChangeEngine |
| 9 | apply_changes | W | ✅ | transactional mutation | ChangeEngine |
| 10 | sandbox_exec | W | – (sandbox-internal) | read-only exec | fail-closed |
| 11 | sandbox_test | W | – | tests + verification | structured |
| 12 | sandbox_build | W | – | build + verification | structured |
| 13 | sandbox_status | R | – | runtime status | safe |
| 14 | impact_analyze | R | – | structural impact | bounded traversal |
| 15 | reindex | W | ✅ | full reindex | guarded |
| 16 | repository_health | R | – | health report | bounded |
| 17 | consult | W (external) | – | provider opinion | user-initiated |
| 18 | context | R | – | context packet | 256 KiB |
| 19 | remember | W | ✅ | persist record | evidence-gated |
| 20 | forget | W | ✅ | retire record | confirm-gated |
| 21 | recall | R | – | history evidence | bounded |
| 22 | learn | R/W | ✅ (mutating) | hypotheses | cautious |
| 23 | skill | R/W | ✅ (mutating) | lifecycle | gated |
| 24 | task | R/W | ✅ (mutating) | task runtime | fenced |
| 25 | engineering_brief | R | – | decision support | bounded |

All 25 router-pinned (`all_tools_have_router_entries` test + live `tools/list` e2e assertion `mcp_lists_exactly_25_tools_and_no_crud`). Every mutating tool acquires the per-workspace mutation lock as designed (verified lock acquisitions at each mutating handler head). No redundant tools (each capability-oriented tool covers a distinct user-facing capability; `context` vs `engineering_brief` differ — always-available packet vs task-scoped decision support; `engineering_memory` vs `memory_stats` differ — resolution vs stats). No accidental CRUD: no tool exposes arbitrary SQL, table rows, or storage internals; no getter returns row ids or raw payloads (adversarially pinned). No tool should be removed. **PASS.**

## 14. MCP Semantic Quality

Tools expose semantic engineering operations: `engineering_brief`, `context`, `recall`, `learn`, `skill` (lifecycle actions), `task` (lifecycle actions), `impact_analyze`, `repository_health`, `reindex`, `engineering_facts` — all capability-shaped with domain vocabulary, never table-shaped. OpenCode never learns CodeBro's internal schema (schema is invisible in every response: canonical prefixed ids only, no table/column names, no SQL fragments). `apply_change(s)` operate on text patches with guards, not file handles. Sandbox tools return evidence envelopes, not raw streams. **PASS.**

## 15. Boundary Between MCP and Domain

MCP handlers are thin adapters: authority assignment happens in the semantic write layer (`remember` handler → store gates), redaction at write seams + store seams (not in handlers ad hoc), scope checks in `resolve_workspace` + store-level workspace keys, ranking in domain runtimes (facts ranker, memory resolver, fingerprint resolver, recall ranker), task transitions in the store matrix (callers never set status), skill lifecycle in the skills store (approve gates: validated status + confidence floor + user_confirmed speech act). No handler constructs SQL or duplicates domain policy (grep-verified: no `rusqlite` in `mcp/mod.rs` outside tests; all DB access flows through `context-runtime` runtimes). **PASS.**

## 16. Determinism

Briefs: byte-equality on repeat assembly, reordered-keyword equality, concurrent agreement, restart byte-equality (unit + adversarial + real-binary E2E + this gate's fresh restart probe). Facts: deterministic lexical scoring with kind/name/path tiebreaks. Memory: importance→confidence→id ordering. Recall: BM25 + deterministic priors (task match → kind importance → recency → id); FTS row order never leaks (deterministic rank applied after retrieval). Impact/health: sorted emission everywhere (`sort_by` on ids/locations before caps). The only `HashMap`s on composition paths are membership/adjacency maps with sorted emission. `now` enters only through documented stateful fields (lease TTL, freshness, decay) — state changes, not nondeterminism. Random-looking ids use diffuse mixing of (time, pid, counter) but are existence-checked — uniqueness, not ordering, is their contract; no ordering depends on them. **PASS.**

## 17. Boundedness

Every retrieval/composition path has explicit caps: facts limit ≤50 (server-clamped — gate probe confirms 100000 clamps, not errors), memory ≤20 entries/500 tokens, context packet 8 records/256 KiB, recall ≤50, learning list ≤50, skills discover bounded, task list ≤100, impact max_nodes 1000/depth ≤5, health ≤500, and the brief's 20+ explicit per-section caps (§28 of P7 audit, re-verified) + 256 KiB envelope with structural windowing. Gate probes: 1 MB task string and absurd limits degrade gracefully (bounded responses, no panic). Massive-store adversarial test (300 symbols) stays bounded. No unbounded query or unbounded `Vec` growth exists on any MCP path. **PASS.**

## 18. Concurrency

Per-workspace mutation lock serializes all mutating tools in-process (lock-acquisition verified at each handler head; workspace-isolated locks tested). Task leases with fencing (`wkr::` ids, `lease_version` monotonic; takeover via explicit resume only; stale-worker refusal tests). `based_on_version` optimistic concurrency refuses stale writers. Briefs take no lock and perform short read transactions on WAL SQLite — concurrent briefs with reindex/task/skill mutation are test-pinned (no deadlock; each brief well-formed). This gate's fresh probe (`gate_concurrent_brief_during_mutation_stays_consistent`): two concurrent client connections, task transitions interleaved with briefs — briefs observed exactly the committed states (`running` → `paused`), no phantom/fabricated intermediate state. Cross-process single-writer assumption documented (P5 honest limitation #2). No deadlocks, lost updates, lock inversion, or inconsistent reads found. **PASS.**

## 19. Task Boundary

P5 is durable state, not execution — verified: no code execution, no scheduling, no workers, no model calls, no skill execution anywhere in `tasks.rs` (the runtime mints worker ids as ownership labels only). Tasks persist state, checkpoint immutably, lease with fencing, resume explicitly, expose bounded snapshots. Completion gate requires a recorded passed validation. P7 did not couple brief generation to task mutation: `task_resume_snapshot` is read-only (test-pinned: status/version unchanged after a brief; workspace write-nothing E2E). Interrupted tasks never auto-complete (restart probe: killed server's running task still reads `running`). **PASS.**

## 20. Skill Boundary

P4 manages the lifecycle (discover/propose/inspect/validate/approve/reject/deprecate/rollback/health); CodeBro never executes skills — no code path runs a SKILL.md, and applicability information never implies execution (brief `skills[]` entries are applicability metadata only). Approval requires the explicit `user_confirmed=true` speech act plus store gates (validated, confidence ≥0.60, workspace match, no name conflict, no stale anchor) — the model can never self-approve. Deprecation removes the artifact; rollback is deterministic version republication; publication is atomic/symlink-safe/read-before-write. Workspace isolation enforced on every action (`skill_candidate_visible_from`). P7 audit F1 fixed skill-description redaction at propose; re-verified this gate. **PASS.**

## 21. Learning Boundary

P3 requires evidence: candidates form only from ≥3 real, same-scope history events (existence-checked; fake/foreign evidence dropped); chatter/sensitive topics never mined; secrets in history never reach propositions. Accepted hypotheses persist as AI_INFERRED only (never USER_CONFIRMED — forged self-confirmation refused at store and MCP layers; gate re-probe). Confidence is bounded [0.05,0.95], decaying, 180-day TTL, evidence-weighted. Repeated AI inference cannot bootstrap itself into high authority: rank 10 < observed 20 < … < confirmed 100, and only explicit user confirmation (speech-act flag) can promote via supersede. Contradictions visible (contested ⇒ deferred; majority-against ⇒ rejected); rejected knowledge preserved as negative evidence that survives re-runs. **PASS.**

## 22. Repository Intelligence Boundary

P6 is deterministic: tree-sitter is the only symbol source; unsupported languages (C/C++/shell/TOML/YAML/JSON/markdown) get file-level intelligence with explicit `parser_limitation` and **zero invented symbols** (e2e-pinned); unsupported stays UNSUPPORTED, unresolved stays UNRESOLVED, stale stays STALE. No repository code execution (parsers operate on file bytes; sandbox tools are separate and explicit), no repository modification (index writes only `.codebro/facts.json` + its own SQLite metadata row), no LLM dependency. Freshness computed from generation-state vs current-state hashes; absent signals report UNKNOWN, never fabricated. **PASS.**

## 23. Engineering Brief Boundary

P7 retrieves, ranks, compresses, and exposes uncertainty — it does not decide. Structurally: no decision/recommendation/solution/plan/next_step fields exist on `EngineeringBrief` (type-system verified). `task_state.next_action` is the task's own checkpoint evidence, not a brief instruction. Language scan (this gate's fresh probe + the adversarial suite) proves no imperative decision text appears. Ambiguity surfaces as AMBIGUOUS_TARGET with bounded candidates — never a guess. Constraints carry hardness + authority (identity and user-confirmed = hard; others observed); risks are signals with sources. The brief contains enough evidence for OpenCode to reason (impact + tests + history + memory + learning + constraints + decisions + freshness) and never tells it what to implement. **PASS.**

## 24. Memory / History / Learning / Facts / Decisions / Experience

Semantic separation verified:

- **History** (P2, `events`): "what happened" — immutable append-only evidence with provenance. Never inference.
- **Learning** (P3): "what tends to happen" — hypotheses with confidence, authority-capped at AI_INFERRED, evidence-cited to history. Distinct rows, explicit promotion path only.
- **Engineering memory** (JSON store): agent-recorded durable context with confidence scores — explicitly NOT verified truth (AGENTS.md hard rule; fact store is separate).
- **Facts** (`facts.json`/FactStore): verified repository structure (tree-sitter provenance) — never execution evidence.
- **Decisions** (identity + records): "what was chosen" with currency and supersede chains.
- **Experience/preferences** (context records): "how the user works" — fingerprint-resolution, never promoted to constraints.

No dangerous semantic duplication: each lives in its canonical home with distinct write paths and distinct read semantics. The brief composes them with per-section categories and authorities so they cannot collapse (adversarially pinned). **PASS.**

## 25. Negative Knowledge

SUCCESS/FAILURE/REJECTED/SUPERSEDED handled without inversion: failed historical approaches feed `failed_task`/`failed_validation` negatives and failure-pattern learning (as AI_INFERRED evidence with confidence, never a hard prohibition); rejected learning surfaces only as `rejected_learning` negative entries; superseded decisions keep `current:false`. A failure becomes a prohibition only if the user explicitly confirms a constraint record — no automatic path exists (gate conflict probe: confirmed constraint + competing evidence all surface distinctly). Negative evidence is relevance-filtered like everything else. **PASS.**

## 26. Freshness

Live (fresh/stale/unknown via generation-hash comparison) and persisted (READY/STALE/FAILED/UNKNOWN + indexed_at + revision in `repo_indexes`) both reported; STALE ⇒ `STALE_INDEX` unknown + risk signal labeling structural evidence as last-indexed state; FAILED preserves last-good metadata (`failed_index_upsert` — only status moves; counts/revision retained). Stale data never presents as current (e2e: stale brief asserted NOT to contain a post-index symbol). Full cycle tested (index → brief → modify → stale brief → reindex → changed brief → restart). Absent rows read UNKNOWN, never READY. `repo_indexes` written only by MCP `reindex` (documented carried limitation — CLI init does not write the row; freshness then reports unknown, never fabricated). **PASS.**

## 27. Failure Model

Every partial failure degrades to explicit unknowns, never silence or false certainty: corrupt state.db → brief answers with unknowns (adversarial); history/FTS unavailable → NO_HISTORY ("unavailable, not absent"); learning/skill reads fail → empty + unknowns; missing identity → MISSING_IDENTITY with defaults; empty store → EMPTY_REPOSITORY; failed reindex → FAILED_INDEX + preserved last-good (adversarial + gate re-probe path); parser failure per-file → deterministic skip with limitation disclosure. Sandbox unavailability fails closed (no silent local fallback). Unknowns are additive — degraded sources each add their marker rather than shrinking the list. **PASS.**

## 28. Error Semantics

MCP errors distinguish invalid input (`invalid_params` with guidance — empty scope, blank targets, `..`, NUL, depth >99, unknown kinds/actions), not found (stable NotFound shapes: cross-workspace task = TASK_NOT_FOUND with no existence leak), stale (stale-writer/lease refusals naming the condition), conflict (fresh-namespace clash naming incumbent; read-before-write skill conflicts), unsupported (UNSUPPORTED_LANGUAGE), lock/deadlock conditions surface as refusals with reasons, persistence failures as storage errors. Internal filesystem/DB details do not leak beyond bounded one-line error excerpts (240-char truncated, redacted). Gate probes confirm malformed inputs (NUL, 1 MB strings, absurd limits) produce clean rejections, never panics. **PASS.**

## 29. Security

Path traversal: ChangeEngine rejects literal `..`, canonicalizes, and re-checks at apply time (symlink-swap refusal); brief targets reject `..`/NUL. Workspace traversal: forged roots canonicalize to the caller's namespace; symlinked aliases collapse to canonical roots. Task/skill id manipulation: opaque ids, existence-checked, workspace-gated on every read. Secret leakage: §12 (PASS after re-attack). Log leakage: tool output redaction is the same single authority; history redacts at the store seam; MCP responses carry no row ids/payloads/digests (adversarially pinned). Input abuse: bounds server-side-clamped everywhere; malformed JSON fails schema deserialization; invalid Unicode rejected at seams; huge inputs bounded (gate-proven). Symlinks: discovery `follow_links(false)`, publication symlink-safe, credentials store refuses symlink targets. Filesystem boundaries: `.codebro/` runtime state only; sandbox execution confined; no writes outside workspace + state dir + skills dir anywhere in P0–P7. No new attack surface introduced by this gate (probe file is test-only, hermetic). **PASS.**

## 30. Real Binary Verification

This gate's independent probes (7 tests, `final_gate_probe.rs`) run against the real `codebro` binary over stdio RPC: tools/initialize handshake, tools/call, tool errors. Covered: secret injection through 5 write paths hunted through 5 read paths (zero leaks); two-workspace zero-leakage with cross-probes of task/context/brief/recall; hard-kill restart persistence (running task survives, never auto-completed) + post-restart brief determinism; malformed input (traversal roots, NUL, 1 MB strings, absurd limits/depth); trust boundaries (evidence-cited rejection; confirm-of-nonexistent refusal); conflicting constraints visible + decision-neutral language; concurrent two-client brief+mutation consistency. Plus the existing real-binary E2E suites (P6 e2e incl. live `tools/list` 25-tool assertion, P7 e2e 5-leg freshness/restart spine, task runtime e2e restart/isolation legs, P7 adversarial 16). Not relied on unit tests alone. **PASS.**

## 31. Test Quality

Suite: hermetic (tempfile::tempdir for all filesystem tests; explicit `CODEBRO_STATE_DIR` for every state-db-touching test; `CODEBRO_SKILLS_DIR` isolation wherever publication is exercised — the shared-env-var test group serializes under one lock with set/remove discipline), deterministic (repeated-pass determinism tests; no time-dependent assertions outside documented lease/freshness state), meaningful (behavioral lifecycle/isolation/bounds assertions, not happy-path snapshots), adversarial where needed (16 P7 adversarial tests + gate probes). No test depends on `~/.codebro` (md5-verified before/after full suite: state.db + memory.json byte-identical; the P2 lesson is institutionalized). No order dependence (workspace tests use per-test tempdirs; env-var tests serialized). The credentials chmod test failure under root is **PRE-EXISTING** (root ignores 0o555) and **NOT P7/gate-relevant** — passes as non-root uid 1000 (re-verified this gate). **PASS.**

## 32. Documentation

AGENTS.md, MCP_API_V1.md, CHANGELOG.md, and the eight evolution docs (P0–P7 implementation + post-implementation audits + phase plan) checked against code: 25-tool count consistent everywhere current (P5/P6 docs' "24 tools" is historically accurate at their time), schema v7 consistent, redaction guarantees match the implemented seams, mutation-lock list matches the code, store separation (state.db vs JSON stores) matches the crate layout. Tool-count docs match live behavior. No concrete drift found this gate. **PASS.**

## 33. Known Debt (unchanged, non-blocking)

1. Task↔skill association is read-time resolution, not an association table (why: P6 chose reference-only refs to avoid a second source of truth; severity: low; risk: bounded by MAX_TASK_SKILL_REFS + workspace gating; blocks release: no; address before P8: optional).
2. `task_id` namespace mixing across seams (opaque strings, workspace-keyed reads; no correctness failure observed; document, defer).
3. Lifecycle vocabulary sprawl (IndexStatus has 6 states, only READY/FAILED persisted, STALE computed, DISCOVERING/INDEXING never written — documented INFO; no enforcement theater added).
4. `engineering_memory.json` (JSON) vs SQLite store separation (deliberate: agent memory is not verified truth, facts are per-project; brief reads the JSON store through the canonical runtime; not worsened).
5. `repo_indexes` written only by MCP `reindex`, not CLI init (freshness reads unknown, never fabricated; document).
6. Brief depth-0 floors to 1 (documented; `impact_analyze` owns depth 0).
7. Record-keyword enrichment (handler) and scope-keyword enrichment (assembler) are sibling sets, not identical (documented; no trust consequence).
8. Task-ref resolved skills carry `version: 0` (P6 read-time resolution design; cosmetic).
9. Wall-clock lease TTL (15 min) — skew shifts takeover windows only; fencing arbitrates (documented).
10. Cross-process single-writer assumption (documented; in-process fully serialized).
11. Brief echoes caller's own ad-hoc task text unredacted (same-channel echo, never persisted; convention).
12. Docs layout: `docs/` contains pre-MCP legacy reports alongside evolution docs (cosmetic; LEGACY_RETIREMENT.md explains).

None compromise safe use as engineering infrastructure. **All deferred intentionally.**

## 34. Architectural Complexity

Major conceptual systems: fact store, context store (SQLite), memory runtime (JSON), identity runtime (JSON), impact engine, indexer pipeline, change engine, sandbox runtime, 4 composition surfaces (context packet, brief, doctor, root-cause), 25 MCP tools. Count is proportional to the capability surface — no duplicate engines, no duplicate stores (each store has one canonical home; FTS5 rows are derived-in-transaction, not a second store), no duplicate ranking (brief inherits source rankings; facts/memory/recall each own exactly one ranker), no duplicate identity (RepoIdentity is the sole authority; repo_indexes caches a bounded redacted projection), no duplicate lifecycle models (record lifecycle vs task lifecycle vs skill lifecycle are genuinely different domains), no speculative interfaces (all seams have live callers; declared extension points — unused history kinds, library-only kernels — are documented INFO, not hidden dead weight), no dead compatibility layers (v1.0.0 memory stores load unchanged — backward compat is 12 lines, not a layer). The only complexity worth noting is the three-lifecycle spread, which reflects three genuinely distinct domains. No concrete architectural problem found. **PASS.**

## 35. P0–P7 Data Flow

OpenCode → MCP (`engineering_brief`, task-scoped) → workspace resolution (canonical root) → task read (workspace-gated, read-only snapshot) → evidence retrieval (facts search bounded, memory resolver budgeted, recall session-grouped, learning list bounded, skills registry + task refs, identity decisions/constraints) → repository intelligence (impact one-traversal bounded, health filtered, freshness live+persisted) → brief assembly (typed sections, per-section caps, deterministic sorts, authority/scope/freshness/provenance preserved verbatim, unknowns additive) → 256 KiB envelope → OpenCode decides. Verified: scope preserved (workspace keys at every read), authority preserved (verbatim, ranked never upgraded), freshness preserved (live+persisted+unknowns), provenance preserved (category+provenance on every section), bounds preserved (caps + envelope), uncertainty preserved (unknown taxonomy). No layer destroys metadata; the only transformations are documented excerpts with truncation markers. **PASS.**

## 36. Realistic Scenarios

- **A (leaf bug fix):** gate conflict probe + P7 usefulness tests — keyword discovery → relevant files/symbols, impact on target, tests honesty. Bounded, deterministic, decision-neutral. ✅
- **B (high-fanout core):** adversarial massive-store (300 callers) — direct edges capped 10 + totals + truncated signal + blast radius. ✅
- **C (refactor with task state):** task runtime e2e — checkpoint/start/pause cycles, restart durability, brief reads state read-only (gate concurrency probe shows exact committed states). ✅
- **D (skill-assisted workflow):** P4 e2e propose→validate→approve→v2→rollback→deprecate + P7 applicability (never execution). ✅
- **E (previous failure + learning):** failure-pattern learning tests + negative-knowledge surfacing in briefs. ✅
- **F (conflicting constraints):** gate probe — user constraint + identity decisions + competing evidence all surface with distinct authority; no resolution invented. ✅
- **G (stale index):** P7 e2e leg — modify → stale brief (no new symbol) + STALE_INDEX unknown + risk signal. ✅
- **H (reindex fails):** adversarial FAILED_INDEX + last-good preservation + gate corrupt-db degradation. ✅
- **I (two similar workspaces):** gate zero-leakage probe (shared state dir, distinct repos). ✅
- **J (large repo, ambiguous target):** AMBIGUOUS_TARGET + bounded candidates + skipped traversal. ✅
- **K (secret-containing input):** gate five-write-path secret attack → zero leaks in all read paths. ✅
- **L (concurrent brief + mutation):** gate two-client probe — no deadlock, exact committed states. ✅

All twelve scenarios remain relevant, bounded, deterministic, scoped, provenance-aware, uncertainty-aware, decision-neutral. **PASS.**

## 37. Release Readiness

| Dimension | Assessment |
|---|---|
| Correctness | 1383/1383 as non-root; clippy -D clean; fmt clean |
| Security | write-seam redaction + projection defense re-attacked; PASS |
| Isolation | two-workspace zero-leakage re-proven; PASS |
| Persistence | restart/atomicity/FTS-sync verified; PASS |
| Determinism | byte-stability incl. restart re-proven; PASS |
| Boundedness | explicit caps everywhere; 1 MB inputs degrade gracefully; PASS |
| Concurrency | locks/leases/fencing; fresh concurrent probe; PASS |
| Maintainability | thin handlers, one redaction authority, DAG deps enforced; PASS |
| MCP quality | 25 semantic tools, no CRUD, router-pinned; PASS |
| Architectural coherence | phase boundaries hold; composition-only P7; PASS |
| Documentation | consistent with code; PASS |
| Test coverage | adversarial + e2e + hermetic; PASS |

## 38. Schema

**v7 remains correct.** No persistence-correctness requirement emerged: this gate added zero product code; every fix-class verified (redaction, isolation) was already at the write seam within existing schema; FTS sync is transactional; migrations remain crash-safe and tested. A schema bump would be churn without a correctness driver. Documented as sufficient.

## 39. MCP Surface

**25 tools — confirmed.** Live `tools/list` e2e assertion, router-pinning test, and this gate's real-binary probes all agree. No additions or removals made by this gate (none required).

## 40. Findings

| ID | Sev | Finding |
|----|-----|---------|
| GF-1 | INFO | Probe-design lesson (not a product defect): the brief legitimately echoes caller-supplied `task` text and `keywords` (same-channel caller echo, never persisted). Any auditor hunting cross-workspace leaks must not embed the hunted marker in the probing request itself. Documented convention; no action. |
| GF-2 | INFO | `update_identity` requires an existing identity (created by `codebro init`/`reindex`); calling it on an un-indexed workspace errors cleanly. Correct behavior, documented here for completeness. |
| GF-3 | LOW | Brief depth floors 0 → 1 (documented debt #6; inherited F3 from P7 audit; doc corrected then). |
| GF-4 | INFO | Credentials chmod test fails only when the suite runs as root (root ignores 0o555). PRE-EXISTING, NOT P7, NOT gate-relevant; passes as non-root (uid 1000 verified). Classification unchanged. |

Zero CRITICAL, zero HIGH, zero MEDIUM. Gate-probe design false-positives (two) were resolved as probe bugs, not product defects — each was verified against documented same-channel-echo behavior before dismissal.

## 41. Fixes

**None required.** The gate found no release-blocking defect. No product code was changed by this gate (fix policy honored: cosmetic/speculative items untouched).

## 42. Remaining Debt

All twelve documented debts (§33) remain, explicitly classified, none blocking, all intentionally deferred. No new debt introduced by this gate (the probe file is additive test-only code with its own regression value).

## 43. Final Verdict

**RELEASE READY WITH NON-BLOCKING DEBT.** 0 unresolved CRITICAL, 0 unresolved HIGH, no trust violations, no isolation leaks, no persistence corruption, no architectural boundary breach, deterministic behavior where promised, bounded behavior, security PASS, concurrency PASS, real binary PASS, P0–P7 regression PASS (1383/1383 as non-root; 1376/1376 baseline + 7 gate probes). All remaining debt is explicitly documented and does not compromise safe use of CodeBro as engineering infrastructure for OpenCode.

**P8: NOT STARTED.** This gate performed no P8 work, added no product features, expanded no architecture.
