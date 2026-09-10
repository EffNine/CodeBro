# P0 Post-Implementation Review — Persistent Context Foundation

Date: 2026-09-06. Scope: the P0 working tree (`crates/context-runtime/` (new),
`context` MCP tool, `engineering_context.rs` records integration) judged
against `docs/evolution/PHASE1_AUDIT.md`, `PHASE2_GAP_ANALYSIS.md`,
`PHASE3_ARCHITECTURE.md`, `PHASE4_PLAN.md`.

## Executive Summary

**Verdict: PASS WITH CHANGES** — three defects were found during this review,
fixed with minimal patches, and covered by new regression tests (all green).
With those fixes, P0 is a durable foundation and **P1 may begin**.

| # | Finding | Severity | State after review |
|---|---------|----------|--------------------|
| F1 | `supersede_record` never wrote the replacement's FTS5 row — every confirmed record became invisible to keyword search | Foundation-invalidating | **Fixed** (`store.rs`), regression test added |
| F2 | `open_with_recovery` quarantined (renamed aside) the database on *any* error, including lock contention / IO / permissions — a healthy `state.db` could be moved aside under load | Dangerous | **Fixed** (`should_quarantine` gate in `db.rs`), classifier + live tests added |
| F3 | Quarantine moved `state.db` + `-wal` but left `-shm` behind, contradicting its own documented contract | Correctness / hygiene | **Fixed** (`db.rs`), existing test extended |
| F4 | No positive FTS search test existed; the FTS happy path was unproven (this is how F1 slipped through) | Test gap | **Fixed** (baseline test added) |

No P1 work was started. No architecture was redesigned. All fixes are
surgical (one line, one gate function, one sidecar path) inside the P0
footprint.

---

## 1. P0 Objective

Per `PHASE4_PLAN.md`, P0 ("Persistent Context Foundation") was to deliver,
as one independently testable increment:

1. New crate `crates/context-runtime` (deps: `codebro-core`, `rusqlite` 0.31
   bundled, plus serialization/error crates), wired into the workspace and
   the dependency-direction script (`context-runtime → core` only).
2. Domain types: `Authority`, `RecordKind`, `RecordScope`, `RecordStatus`,
   `LifecycleStage`, `ContextRecord`, `Event`, `Session`, plus a
   confidence-decay function.
3. Store: `~/.codebro/state.db` bootstrap (WAL, `user_version` migrations,
   quarantine-on-corruption); tables `context_records` / `events` / `sessions`
   + FTS5; insert/update/supersede/expire-sweep/list/query; FTS5 row sync in
   the same transaction; bounded payloads.
4. Retrieval: `ContextQuery` + deterministic ranking behind a trait seam
   (`ContextRetriever`).
5. MCP: `context` tool — the always-available packet (existing
   `engineering_context` composition + records section), provenance-tagged,
   bounded via `response_bounds`, optional task (absent = structural digest),
   per-workspace scoping.
6. Tests: migration, corruption quarantine, authority gates, lifecycle,
   supersede/expire, FTS5 sync, restart persistence, composition bounds,
   no-files-created, isolation from facts/memory, two-writer serialization.
7. Docs: `AGENTS.md` tool table, `CHANGELOG`, evolution records,
   `MCP_API_V1` additive note.
8. Standing constraints: no changes to existing tool shapes or `.codebro`
   JSON semantics; no embeddings/network/agent loops; `tempfile::tempdir()`
   for filesystem tests; `CODEBRO_STATE_DIR` override for hermetic tests.

## 2. Actual Implementation

What the working tree contains (vs the frozen `HEAD` baseline):

- `crates/context-runtime/` (new, ~1800 lines incl. tests): `lib.rs`
  (scope/invariants, `context_runtime` facade), `types.rs` (domain model +
  validation), `db.rs` (bootstrap/migrations/quarantine), `store.rs`
  (transactional `ContextStore` + `ContextRetriever` impl), `retrieval.rs`
  (trait, `RecordQuery`, `RankedRecord`, decay, tokenizer).
- `crates/mcp-server/src/mcp/mod.rs`: `context_store: Arc<ContextStore>`
  on the server, `assemble_server` with optional `state_dir`,
  `default_state_dir()` (`CODEBRO_STATE_DIR` → `~/.codebro`), `context`
  tool (tool 18) + `ContextArgs` + 5 integration tests.
- `crates/mcp-server/src/engineering_context.rs`: records-aware
  `compose()` / `compose_structural()` (new `records` section,
  `ContextRecordExcerpt`, `excerpt_from`, `MAX_CONTEXT_RECORDS = 8`,
  `MAX_RECORD_CONTENT_CHARS = 240`), existing sections untouched.
- Wiring: workspace `Cargo.toml` + `mcp-server/Cargo.toml` +
  `scripts/check_workspace_deps.sh` all updated; `AGENTS.md`,
  `CHANGELOG.md` (Unreleased P0 entry), `docs/MCP_API_V1.md` (additive
  `context` row) updated; `docs/evolution/` (4 phase docs) added.
- This review's fixes: `sync_fts` on the supersede path, `should_quarantine`
  gate, `-shm` quarantine hygiene, 4 new/extended tests (see §15).

## 3. Architecture Review

### 3.1 Implemented correctly

- **Crate placement and direction.** `context-runtime` depends only on
  `codebro-core` among workspace crates (`crates/context-runtime/Cargo.toml`);
  the dep script allows exactly that, and `mcp-server` is allowed to consume
  it. Composition lives in `mcp-server` (thin handler delegates to
  `engineering_context::compose` + store query), consistent with the
  thin-adapter rule. No cycles, no MCP coupling inside the runtime.
- **One home per domain.** Facts/decisions/constraints stay in their JSON
  stores; context records reference them via `related_ids` only. The
  `context` read path never writes `.codebro/*.json` (pinned by
  `context_composition_never_touches_project_state_files` and
  `composing_creates_no_files`).
- **Additive contract.** All 17 existing tools byte-identical; `context` is
  purely additive with optional args only. `MCP_API_V1.md` records it as a
  minor-version addition. No `.codebro` JSON semantics changed.
- **No second reasoner.** Everything in P0 is deterministic storage,
  validation, and ranking. No LLM calls, no network, no embeddings, no
  agent loop, no executor, no planner. The learning loop is correctly
  deferred to P3 (only the event log it will consume exists).

### 3.2 Implemented differently (accepted deviations)

| Planned (PHASE3) | Actual | Assessment |
|---|---|---|
| `project_id` (xxHash of root, `RepoIdentity` scheme) as scoping key | Plain `workspace_root TEXT` scoping | Acceptable simplification, but rename/move-fragile and un-normalized at the store layer (only the MCP caller canonicalizes). P1 must define canonicalization for `remember` (see §15). |
| "BM25 blended with the existing deterministic lexical scorer" + "authority/decay weighting" | BM25 order only for keyword queries; importance/`updated_at` order otherwise; decay computed as *metadata*, never used in *ordering*; authority influences nothing in ranking | Weaker than specified. Acceptable for P0 (deterministic, swappable seam exists), but the architecture doc over-claims. Retrieval is currently **A-minus: good foundation, primitively ranked** (see §6). No rewrite needed. |
| `USER_CONFIRMED` "requires explicit caller flag" | Store accepts any authority from any caller; no caller-identity concept | Correctly deferred: there is no write tool in P0, so there is no principal to check. This gate **must** land with P1's `remember` (see §13). |
| "≥3 repeated evidence" / evidence freshness driving decay | Decay is pure wall-clock (`rate^months`); evidence ids are opaque strings, never resolved to real events | Acceptable for P0, but §5 details the trust consequence. |
| Hierarchy resolution task > project > global per `kind_namespace` | Composer returns top-8 by importance regardless of scope/kind; no per-namespace winner | Correctly deferred to P1, but P1 must not just "add more records" — it needs the resolution rule (see §13). |
| `Task` scope | Exists as an enum value with no task identifier — a task record is indistinguishable from another task's record in the same workspace | Half-built. Either P1 gives Task records a task binding (or expiry), or Task scope should not accept writes until then (see §15). |

### 3.3 Omitted (correctly)

`remember`/`forget`/`recall`/`learn`/`skill`/`task` tools, session
open/close APIs (table present, methods absent — honest reservation),
passive event capture in handlers, hierarchy resolution, redaction pipeline
(no free-text user-input path exists yet to redact), embeddings (correctly
rejected; Hermes' own vector store was empty in practice per the gap
analysis).

### 3.4 Over-engineering found

1. **`LifecycleStage` duplicates `Authority`** (`types.rs:253-288` vs
   `:38-80`). `Observed→Inferred→Confirmed` is the same axis as
   `Observed→AiInferred→UserConfirmed`, but nothing links them: a record
   can legally be `authority: UserConfirmed` + `lifecycle: Observed`.
   Two parallel hierarchies with no consistency rule is a future migration
   trap. Recommendation (§16): P1 should document the invariant
   (authority determines lifecycle floor) or collapse them — do not grow a
   third status-like field.
2. **`Fact` / `Decision` / `Skill` kinds are "reference-only" by comment
   only** (`types.rs:107-111`). Nothing requires `related_ids` or forbids
   duplicating canonical content under these kinds. Either enforce the
   reservation in `validate_record` or drop the variants before callers
   depend on them.
3. **A second provenance vocabulary.** `ContextProvenance`
   (`verified/recorded/observed/derived`) sits beside core `SourceKind` and
   the new `Authority`. The mapping is documented in a comment
   (`engineering_context.rs:78-95`) but not enforced, and per-item trust is
   uneven: record excerpts carry `authority`, but memory excerpts and
   decisions carry only the section-level `recorded` tag. Trust boundaries
   are *partially* visible (see §7).
4. **Unused `uuid` dependency** in `context-runtime/Cargo.toml` — referenced
   only by a doc comment (`types.rs:293`); ids are caller-supplied strings.
   Remove before it invites a second id scheme.
5. **Dormant schema surface**: `sessions` table + `EventRecord.session_id`
   with zero methods. Fine as a reserved migration step (it is versioned
   under `user_version = 1`), but P2 must design session APIs against this
   exact shape or migrate it — flag it explicitly in the P2 kickoff.

### 3.5 Under-engineering found (fixed or filed)

F1–F4 (fixed, see Executive Summary and §15); evidence-existence,
cross-process writer tests, and context-runtime write-isolation guard
(filed as required/recommended follow-ups, §15–§16).

## 4. Persistence Review

### 4.1 Confirmed characteristics

- Canonical path `~/.codebro/state.db` with `CODEBRO_STATE_DIR` override
  (`mcp/mod.rs:83-89`); user-level store, per-workspace rows via
  `workspace_root`. All filesystem tests use `tempfile::tempdir()`.
- WAL + `busy_timeout` 5s + `synchronous NORMAL` + `foreign_keys ON`
  (`db.rs:82-85`). Single connection per process behind a `Mutex`
  (`store.rs:38-41`), mirroring the server's `mutation_lock` discipline.
- Sequential `PRAGMA user_version` migration runner, corruption quarantine
  (core naming pattern), newer-schema refusal, lazy opening (constructor
  performs no I/O), transactional writes (record + FTS5 in one `tx`),
  restart persistence (tested), workspace isolation (tested at MCP level).
- FTS5 `unicode61` index over `(content, namespace, original_text)` with
  `record_id UNINDEXED`; tokenizer lowercases and drops <3-char tokens;
  MATCH expressions are quoted literals (`store.rs:563-567`), so no FTS5
  injection via keywords.

### 4.2 Defects found and fixed

- **F2 — quarantine on any error.** `open_with_recovery` previously
  quarantined on every failure mode, including lock contention and IO
  errors. Fix: `should_quarantine()` (`db.rs`) — quarantine only on
  `DbError::Corrupt` (failed `quick_check`) and SQLite
  `DatabaseCorrupt` / `NotADatabase`. Everything else (busy, locked,
  readonly, cannot-open, disk-full, misuse, IO, unusable path) propagates
  untouched; the `context` tool already degrades to an empty records
  section on error. Verified: garbage files still quarantine
  (`corrupt_file_is_quarantined_and_recreated`,
  `corrupt_underlying_file_recovers_transparently` pass unmodified), while
  a non-database obstruction (directory at the db path) is now left in
  place and surfaced as an error (`non_corrupt_open_failure_leaves_files_untouched`).
- **F3 — `-shm` sidecar.** `quarantine()` moved `state.db` + `-wal` while
  its own comment promised all three. Fixed; extended
  `wal_sidecars_are_quarantined_together` covers `-shm`.

### 4.3 Remaining risks (no change — by design or deferred)

- **Future migrations.** `migrate()` is a single idempotent v1 batch, not a
  stepwise runner. That is correct for `SCHEMA_VERSION = 1`, but the first
  P4/P5 table addition must convert it to `match version { 1 → 2, … }`
  steps — never "re-run the whole batch and bump". Noted for the P4/P5
  kickoff; no action now.
- **Adoption hazard.** A valid-but-foreign SQLite file at the state path
  would be silently adopted (tables added, version set). Negligible in
  practice (fixed path under CodeBro's own dir); do not add detection
  machinery for it.
- **Async blocking.** Store calls run blocking SQLite I/O inside the async
  MCP handler without `spawn_blocking`. Fine at current query sizes
  (≤200 rows, ≤8 KiB payloads); revisit if P2 recall adds heavy joins.
- **`parse_list` silent default** (`store.rs:451-453`): corrupt
  `evidence_json`/`related_json` decodes to `[]` instead of erroring. This
  weakens provenance visibility on disk corruption (an `AiInferred` record
  could read back with empty evidence, though the write-time gate held).
  Recommend a strict decode or a `valid` flag when the corruption story is
  next touched — not a P1 blocker.

### 4.4 Durability verdict

With F2/F3 fixed: WAL + busy timeout for multi-process safety, corruption
quarantine that only fires on corruption evidence, newer-schema refusal,
atomic transactions, no silent data loss paths found. **The database can
survive years of incremental evolution** provided future migrations stay
stepwise and additive — the one rule worth writing into the P4/P5 kickoff.

## 5. Provenance & Trust Review

The epistemic core — "User probably prefers X" must never silently become
"User prefers X" — **holds structurally**, with one gap to close in P1.

What holds (verified in code + tests):

- `AiInferred` / `Observed` without evidence are refused at validation
  (`types.rs:459-468`, `inference_requires_evidence`,
  `observed_requires_evidence_user_confirmed_does_not`,
  `invalid_records_are_refused`). The gate lives in the lowest layer
  (`validate_record`), so every future writer inherits it.
- Promotion preserves the audit trail: `supersede_record` requires the
  replacement to name its predecessor, requires the predecessor to be
  `Active`, refuses double-supersede, and performs both writes atomically
  (`store.rs:193-235`, `supersede_keeps_audit_trail`,
  `supersede_requires_active_target_and_matching_link`).
- Terminal states are explicit: `reject_record` (active-only transition),
  `expire_sweep` (time-based, returns count), hard `remove_record` framed
  as cleanup-only with soft transitions preferred.
- Retrieval defaults to `Active`-only; superseded/expired/rejected rows
  stay queryable only when explicitly requested (`retrieval.rs:21-23`).
- Nothing in `context-runtime` references `facts.json`,
  `engineering_memory`, or project-identity paths (verified by inspection;
  the dependency script makes coupling to those crates a build failure).
- Decay is authority-differentiated and tested: `UserConfirmed` 0.98/mo vs
  `AiInferred` 0.85/mo; a year-old inference fades below 0.2 while a
  3-month-old confirmation stays above 0.9 (`retrieval.rs:49-71` + tests).

Gaps (P1 must close — see §13/§15):

1. **Evidence is cited, not verified.** `evidence: Vec<String>` accepts any
   non-empty strings; nothing checks the ids name real events. An agent can
   satisfy the gate with `["ev:1"]` fiction. P1's `remember` must resolve
   cited ids against the `events` table (at least existence; ideally
   workspace match).
2. **No caller principal.** Any holder of a store handle can write
   `UserConfirmed` today. Harmless while no write tool exists; P1's
   `remember` must mint `UserConfirmed` only from an explicit user-confirm
   signal, and `AiInferred`/`Observed` only from agent-observed flows.
3. **No downgrade path exists** (good): there is no API that rewrites a
   `UserConfirmed` record into `AiInferred`. Downgrade can only happen via
   supersede, which leaves the trail. The reverse hazard — inference
   *read as* confirmation — is contained by per-excerpt `authority` tags
   in the packet, but memory excerpts and decisions lack per-item
   authority (see §7).

## 6. Retrieval Review

**Verdict: A — good foundation, primitively ranked.** Deliberately simple,
deterministic, and swappable; do not add embeddings or vector search.

- The `ContextRetriever` trait seam is the right abstraction: P2 recall and
  any future semantic reranker can implement it without touching the domain
  model or the store. `RecordQuery` (workspace/kind/status/keywords/limit)
  covers every P1–P3 access pattern currently foreseeable.
- Keyword path: FTS5 BM25, `ORDER BY rank, id`, limit clamped 1–200. Safe
  tokenization, no injection, deterministic. **Now actually tested**
  end-to-end (F4 baseline test).
- Keyword-less path: `importance DESC, updated_at DESC, id` — deterministic
  and correct for the "fingerprint at session start" use, but importance is
  caller-declared and defaults to 0.5, so in practice this is recency
  order. Fine for P0; P1 hierarchy resolution must replace "top-N by
  importance" with per-namespace winners.
- Honest limitations (documented here so P1 doesn't rediscover them):
  decay and authority are *reported* (`effective_confidence`) but play no
  role in *ordering*; there is no authority weighting, no recency beyond
  the tiebreak, no dedup of same-namespace conflicts. The PHASE3 phrase
  "BM25 blended with the existing deterministic lexical scorer" is
  aspirational — the blend does not exist. Correct the doc when P1 scopes
  retrieval, rather than building the blend prematurely now.

## 7. Context Composer Review

The packet (`engineering_context.rs`) is genuinely useful and disciplined:

- Repository orientation + fact counts always present (even in the
  structural digest); task-relevant facts/decisions/memory/evidence only
  with a task; `compose` *rejects* empty tasks instead of dumping stores;
  the digest labels itself structural in `notes`. This is exactly the
  "small high-value always-on + queryable history" split from the product
  principles.
- Bounds are real, not decorative: per-section caps (10/5/5/5/5/8),
  per-value excerpting with the shared `…[truncated for context budget]`
  marker, global 256 KiB `bounded_response` envelope, `<16 KiB` asserted on
  seeded workspaces. History stays out of the packet (bounded evidence
  summaries only) — Principle 3 respected.
- Trust boundaries are *mostly* visible: every section carries a
  `ContextProvenance` tag; record excerpts carry per-item `authority`,
  `status`, `effective_confidence`, `language`. Two holes: memory excerpts
  (`key/value/confidence`) and decisions (`id/title/status`) have no
  per-item authority, so OpenCode cannot tell agent-recorded memory from
  declared intent inside those sections. Recommend adding the existing
  memory-entry source/confidence metadata through in P1 (non-blocking).
- Responsibilities are clean: the composer loads and excerpts; it performs
  no traversal (impact guidance is pointers + a "call `impact_analyze`"
  note), makes no decisions ("No decisions made" invariant holds).
- P1 fit: User Fingerprint records slot into the existing `records`
  section (8 × 240 chars ≈ 2 KiB — no blob risk); Intent records likewise.
  What P1 must add is *resolution* (per-namespace task > project > global
  winner), not capacity. No composer redesign needed.

## 8. MCP API Review

The `context` tool (`mcp/mod.rs`, tool 18) supports the invisibility goal:

- **High-level capability, not a database API.** Args are
  `workspace_root? / task? / keywords[]?` — no table names, no SQL, no
  status/kind enums leaked. One tool, not one-per-table. The description
  tells OpenCode *when* to call it ("at task start and when context runs
  thin"), which is the entire UX contract for invisible infrastructure.
- **Understandable output.** Repository orientation, counts, excerpts with
  authority strings, `notes` explaining absences ("no execution evidence
  recorded yet" rather than empty arrays). A reasoning model can consume
  this without CodeBro documentation.
- **Read-safety is honest with one nuance.** The tool writes no records and
  no project files (tested), but lazy opening means the first call may
  create an empty `state.db`, and a corrupt store may be quarantined on
  the read path (by design — F2 narrowed this to genuine corruption).
  "Read-only" means *no knowledge writes*, not *no filesystem touch*.
  Accurate enough for the API doc; the AGENTS.md line-70 phrasing is fine.
- **Minor inconsistency (non-blocking).** `keywords` influences
  facts/memory/decisions but the records section is always top-by-importance
  (keywords ignored). Intentional per the code comment (fingerprint present
  before a task narrows relevance), but with a task supplied, keyword
  relevance for records would be strictly better. P1 should thread
  keywords into the records query when a task exists.
- No second tool is needed. No proliferation risk observed: the planned
  `recall`/`remember`/`forget`/`learn`/`skill`/`task` are each
  capability-level, ~7 total. The trajectory is correct.

## 9. OpenCode Boundary Review

P0 stays on CodeBro's side of the line. Verified:

- No agent loop, planner, subagent, skill executor, chatbot, TUI, or
  reasoning engine added. The new code is storage + validation + ranking +
  one read-only composer. `consult` (the only model-adjacent tool) is
  untouched.
- Permissions ride OpenCode's native model (no approval reimplementation;
  read-only tool needs none beyond existing `codebro_*` patterns).
- No transcript capture, no session shadowing of OpenCode's session DB —
  CodeBro records only its own future observable events (schema-reserved).
- One nuance to protect in P1/P3: the `source=message` seam and the
  learning loop must remain *CodeBro-observable-evidence only*
  (tool calls, mutations, verification outcomes), never a second copy of
  the user's conversation. The architecture docs already say this
  (PHASE3 §2); future reviewers should re-check it when `learn` lands.

## 10. Hermes Comparison

Evaluated from the gap analysis and PHASE1's Hermes-installation findings
(no Hermes checkout lives in this repo, so this judges the extraction, not
the source).

Useful ideas extracted, correctly scoped down:

| Hermes concept | CodeBro P0 treatment |
|---|---|
| SQLite canonical state | Adopted (`state.db`), but *only* for new user-context domains — existing JSON stores deliberately not unified (PHASE3 challenge #1, correctly resolved) |
| FTS5 search | Adopted for records; dual trigram index correctly skipped (unicode61 suffices at this cardinality) |
| Bounded always-on context | Adopted with harder bounds (per-section caps + 256 KiB envelope + excerpt markers) |
| Session/history separation | Schema-reserved (`sessions` table, `session_id`), APIs deferred — separation without premature machinery |
| Memory provenance | Exceeded: 6-authority model + evidence gates + decay vs Hermes' flat prose files |
| Skill lifecycle / kanban / cron / delegation | Correctly deferred (P4/P5/P6) or refused (delegation, compaction, per-platform profiles, approval reimplementation) |

Failure modes avoided: autonomous mutation (P0 adds *zero* write tools —
there is nothing that can act without OpenCode driving); prose dumps
(MEMORY.md-style unbounded files replaced by bounded validated records);
silent promotion (supersede audit trail); second agent loop (none).
The one Hermes-adjacent risk P0 briefly carried — destructive recovery on
the read path — was narrowed in this review (F2).

## 11. Test Review

P0's tests prove the architecture's load-bearing claims, with the gaps
below. Counts: context-runtime 38 unit tests (was 34), mcp-server
`context_*` 5 integration tests, composer 9 tests — all passing.

Proven (verified by running): restart persistence, migration idempotence,
newer-schema refusal without quarantine, corrupt-file quarantine +
recreate, WAL sidecar hygiene (now incl. `-shm`), concurrent in-process
writers (8×25), workspace isolation (cross-workspace leak test at MCP
level), authority gates, lifecycle/supersede/expire/reject/remove, FTS sync
on put/remove/**supersede** (new), keyword search happy path (new),
composition bounds, no-files-created (both layers), lazy opening (implicit
— constructors never touch disk; every test builds stores over fresh
tempdirs), read-path degradation on unusable store, non-corrupt failures
left untouched (new).

Still missing (adversarial gaps, ranked):

1. **Cross-process concurrent writers.** Only threads tested. WAL +
   busy-timeout is the right mechanism, but no test opens two *processes*
   (or two connections racing a write txn) against one db. Recommend one
   P1 test with two connections工作的.
2. **Mechanical write-isolation guard for context-runtime.** Memory has
   `trust_separation.rs` (source scan + byte-preservation test). P0 added
   no equivalent pinning that context-runtime cannot touch
   `facts.json`/`engineering_memory.json`/`project_identity.json`.
   Recommend extending `trust_separation.rs` in P1 (cheap, high-value).
3. **Evidence-id existence.** No test writes an `AiInferred` record citing
   a nonexistent event and expects refusal — because the store doesn't
   refuse it (see §5/§13). Test arrives with the P1 gate.
4. **Explicit-status queries.** Superseded/expired/rejected rows are
   queryable by explicit status, but no test exercises that path (only the
   active-default). Cheap to add in P1.
5. **Non-canonical workspace roots.** No test for trailing-slash, symlink,
   or case variants of the same workspace producing split-brain scoping.
   The MCP layer canonicalizes; direct store callers need a documented
   rule (see §15).
6. **Conflict semantics.** Two active records, same namespace,
   contradictory content — currently both surface. No dedup expected in
   P0, but P1 hierarchy resolution needs a specified behavior.

## 12. Complexity Audit

"Minimum complexity required for a durable architecture" — P0 is close.
Keeping vs cutting:

- **Keep:** separate `context_records`/`events`/`sessions` tables (each maps
  to a distinct phase: P1/P3/P2); FTS5 virtual table (the only index that
  makes recall possible without a later migration); evidence digests
  (cheap integrity without trusting blobs); lazy opening (composition
  stays side-effect-free for project files); the trait seam (one trait,
  one impl — the cheapest possible extensibility).
- **Cut/merge (recommended, non-blocking):** unused `uuid` dep;
  `LifecycleStage`↔`Authority` duplication (pick one axis or document the
  invariant); unenforced `Fact`/`Decision`/`Skill` kind reservations;
  overlapping provenance vocabularies (at minimum, thread per-item
  authority into memory/decision excerpts so three vocabularies don't
  become four).
- **Dangerously simple (fixed):** FTS sync on supersede (was missing);
  quarantine policy (was "any error"); sidecar hygiene (was 2 of 3).
- **Dangerously simple (accepted for now, must not persist past P1):**
  evidence strings unchecked; Task scope without task identity; ranking
  without authority/recency. Each is a documented P1 input, not a P0
  rewrite trigger.

## 13. P1 Readiness

**P1 can be implemented cleanly on top of P0. No P0 redesign is required.**
The reasons are concrete, not aspirational:

- Fingerprint attributes map 1:1 onto `ContextRecord`
  (`Preference`/`Style`/`Taste`/`Constraint`/`Principle` kinds +
  `namespace` hierarchy + `original_text`/`language` verbatim layer +
  `Global` scope). Intent maps onto `Intent` kind + `scope` override.
  No schema change needed for either.
- The trust primitives P1's semantics depend on already exist and are
  tested: evidence-required inference, supersede-promotion with audit
  trail, active-default retrieval, per-authority decay.
- The composer already reserves the exact slot P1 fills (`records`,
  8 × 240 chars), and the MCP surface needs no shape change for P1 reads.
- Storage (`~/.codebro/state.db`, WAL, migrations, `CODEBRO_STATE_DIR`
  hermeticity) is proven under restart, corruption, and concurrency.

Minimum P1 contract (the gates §5 deferred — these are P1 scope, not new
P0 blockers):

1. `remember`: `UserConfirmed` only from explicit user-confirm signal;
   `AiInferred`/`Observed` require *existing* event ids (existence check
   against `events`, same-workspace match); canonical statement +
   verbatim `original_text` + `language` tag per the multilingual rule.
2. Hierarchy resolution: per-namespace winner task > project > global;
   project decisions never auto-promote to global (PHASE3 §47 restated as
   code, with tests).
3. Task scope: bind to a task identity (or expiry) before accepting
   task-scoped writes; until then, reject or coerce to project scope.
4. `forget`: specified as reject (reversible) vs remove (cleanup) with the
   existing primitives — no new state machine.
5. Retrieval doc correction: state what ranking actually is (BM25 /
   importance / decay-reported) and specify the P1 ordering rule if
   authority/recency must influence order.

## 14. Future Phase Compatibility

| Phase | P0 enables | P0 constrains | Migration expected |
|---|---|---|---|
| P1 Fingerprint + Intent | Record kinds, authority, scope, decay, language/verbatim fields, composer slot, MCP read path | No resolution rule, no task identity, evidence unchecked | None (pure additions: `remember`/`forget` tools) |
| P2 Sessions + Recall | `events` table + digests, `sessions` table reserved, FTS5, trait seam, cross-store join pattern in composer | No session open/close APIs, no passive capture, sessions shape fixed prematurely? | Possibly additive columns on `sessions` (`user_version` 2); recall is a new tool, no rewrite |
| P3 Learning + Inference | Evidence-cited records, supersede promotion, decay, RCA "never promoted" precedent | Evidence ids are strings (no backlink index); event `kind` free-form (taxonomy needed) | Additive: kind taxonomy as validation, candidate tables if needed |
| P4 Skills lifecycle | `Skill` kind reservation, checkpoint/drift concepts documented | Reservation unenforced (§3.4); publishing target (`~/.config/opencode/skills/`) unvalidated | New tables via stepwise migration; SKILL.md format external |
| P5 Durable tasks | `sessions.context_snapshot_json` as the snapshot pattern precedent | No task table, no state machine | New `tasks` table via stepwise migration; no executor (by design) |
| Later automation | Stable state path + CLI surface to hang verbs on | None | None foreseen |

Readiness matrix: P1 **ready** (gates scoped above); P2 **ready**
(reserve-then-design against the fixed `sessions` shape); P3 **ready**
(needs taxonomy + evidence-index decisions at kickoff); P4/P5 **compatible**
(each needs exactly one stepwise migration + new tools, no rework of P0).

## 15. Required Changes Before P1

All items in this section are **already implemented and verified** in this
review (they were genuine blockers, not preferences):

1. **FTS sync on supersede** (`store.rs`): `sync_fts(&tx, replacement)?`
   after `insert_record_row` in `supersede_record`. Without it, P1's
   confirm flow (inference → supersede → user-confirmed) produced records
   invisible to keyword search — P2 recall and any confirmation UX built
   on search would silently lose confirmed knowledge. Proven by
   `superseded_replacement_remains_keyword_searchable` (failed before,
   passes after) plus the `keyword_search_finds_indexed_records` baseline.
2. **Quarantine only on corruption evidence** (`db.rs`:
   `should_quarantine`). Without it, lock contention or IO/permission
   failures renamed a healthy `state.db` aside. Proven by
   `quarantine_only_on_evidence_of_corruption` (classifier unit tests) and
   `non_corrupt_open_failure_leaves_files_untouched` (live: directory at
   db path errors without moving anything). Existing garbage-corruption
   tests pass unmodified, confirming genuine corruption still recovers.
3. **`-shm` quarantine hygiene** (`db.rs`): `quarantine()` now moves
   `state.db`, `-wal`, and `-shm` together, matching its documented
   contract. Extended `wal_sidecars_are_quarantined_together`.
4. **P1 kickoff inputs (no code, must be in the P1 plan):** evidence-id
   existence check, `remember` caller-principal rule, per-namespace
   hierarchy resolution, Task-scope identity rule, workspace-root
   canonicalization rule for direct store callers. See §13.

## 16. Recommended Changes

Non-blocking; schedule at convenience, preferably inside P1 where noted:

1. Remove the unused `uuid` dependency from `context-runtime/Cargo.toml`.
2. Resolve the `LifecycleStage`↔`Authority` duplication (document the
   invariant or collapse; do not add a third status axis).
3. Enforce or drop the `Fact`/`Decision`/`Skill` "reference-only"
   reservation in `validate_record`.
4. Thread per-item authority into memory and decision excerpts (or drop
   `ContextProvenance` in favor of the two existing vocabularies).
5. Pass `keywords` into the records query when a task exists (§8).
6. Correct the retrieval over-claim in `PHASE3_ARCHITECTURE.md`
   ("BM25 blended… + authority/decay weighting" — not implemented).
7. Fix stale counts: `ARCHITECTURE.md` ("17 tools"), `AGENTS.md`
   ("Ten crates" — now eleven).
8. Extend `trust_separation.rs` to pin context-runtime write isolation
   (§11.2); add explicit-status query tests, cross-process writer test,
   and non-canonical-root scoping tests (§11.1/4/5).
9. Strict-decode (or flag) corrupt `evidence_json`/`related_json` instead
   of silent `[]` (§4.3).
10. Evaluate `spawn_blocking` for store I/O if P2 recall grows query cost
    (§4.3).

## 17. Final Verdict

**PASS WITH CHANGES — P1 is approved to begin.**

P0 delivers what it promised: a minimal, deterministic, provenance-aware
persistence foundation with a bounded, useful, read-only context packet —
and it does so without leaking across the OpenCode boundary, without a
second source of truth, and without speculative machinery. The three
defects found in this review (FTS desync on supersede, over-broad
quarantine, `-shm` hygiene) were each one-line-class fixes with regression
tests, all verified green. The remaining gaps are scoped P1 work with
concrete acceptance rules (§13), not P0 rework.

The foundation is durable enough to evolve for years — provided P1 honors
the gates this review scoped, and P4/P5 keep migrations stepwise.
