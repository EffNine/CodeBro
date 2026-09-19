# CodeBro P6 Implementation — Engineering Intelligence Layer

**Status:** COMPLETE (additive over frozen P0–P5).
**Schema:** v6 → v7 (`repo_indexes` only).
**MCP tools:** 24 (no new tools; existing tools strengthened).
**Architecture:** OpenCode reasons/executes; CodeBro indexes/informs. No
scheduler, daemon, watcher, agent loop, or skill execution.

---

## 1. Mission

Make CodeBro understand a repository as an engineering system, not a
collection of files. P6 provides deterministic intelligence about
repository structure, files, symbols, modules, dependencies,
relationships, architecture boundaries, impact/blast radius, repository
health, relevant tests, and index freshness — all syntactic and
evidence-grounded, never LLM-invented.

Non-goals (explicitly not built): agent loop, code generation,
scheduler/cron/daemon/watcher, model router, embeddings/vector DB,
automatic skill execution, distributed workers, IDE/TUI.

## 2. Existing Engineering Capabilities

Mapped before coding (no duplicate subsystems):

| Capability | Location | P6 reuse |
|---|---|---|
| Immutable `FactStore` (14 kinds) + validation | `crates/fact-store` | Unchanged; graph/health project over it |
| Tree-sitter parsers (Rust, Go, Python, JS, TS) + calls/imports | `crates/parsers/src/intelligence` | Extended file-level only (C/C++/shell/config) |
| `codebro init` pipeline (scan → parse → cache → build → atomic persist) + `file_digests` + `generation_repo_state` | `crates/indexer/src/init` | Reused; new `engineering` module adds pure diff/freshness/architecture |
| `compute_facts_diff` (added/modified/deleted + impact projection) | `crates/indexer/src/init/mod.rs` | Reused conceptually; `engineering::diff_digests` is its pure kernel |
| Impact BFS (depth 0–5, bounded, provenance-weighted) + freshness | `crates/impact-engine/src/impact` | Strengthened with `risk` signal (additive field) |
| SQLite user-context (schema v6, FTS5, 11 tables) | `crates/context-runtime/src/db.rs` | Extended v6→v7 with `repo_indexes` only |
| History taxonomy + recall + learning groups | `crates/context-runtime` | 5 additive `HistoryKind`s; learning explicitly ignores them |
| Skills lifecycle + task runtime (`skill_refs_json` opaque) | `crates/context-runtime` | Read-time resolution only; no write change |
| MCP 24 tools + mutation lock + doctor (7 checks) + context packet | `crates/mcp-server` | Strengthened responses; doctor gains 2 checks |

## 3. P6 Architecture

```text
Repository (source of truth)
    │
    ▼
Discovery (walk, skip-list, size gate)
    │
    ▼
Parsing (tree-sitter where supported; file-level otherwise)
    │
    ▼
Normalization (FileRecord, deterministic symbol IDs, canonical paths)
    │
    ▼
Index (facts.json: symbols/modules/edges + file_digests + generation state)
    │
    ▼
Relationships / Graph (impact-engine BFS over FactStore, no graph DB)
    │
    ▼
Engineering Intelligence (risk, health findings, freshness, architecture)
    │
    ├── repository queries (workspace_context, engineering_facts)
    ├── impact analysis (impact_analyze + risk)
    ├── health analysis (repository_health + findings)
    └── context enrichment (context packet, bounded)
    │
    ▼
CodeBro MCP (24 semantic tools, no CRUD)
    │
    ▼
OpenCode (reasoning, planning, execution, skill execution)
```

Canonical/derived boundary: repository files are canonical;
`facts.json` + `repo_indexes` rows are derived. SQLite never stores
file contents, symbols, or edges.

## 4. Repository Identity

`crates/core/src/repo_state.rs` — `RepoIdentity` strengthened:

- Canonical root (symlink-resolved, lexically cleaned; never raw input).
- VCS identity where available: git remote (`origin` preferred, else
  first) + HEAD SHA (best-effort, `None` outside git).
- `repository_type`: cargo/go/npm/python/unknown (python added).
- `project_id`: stable SHA-256(canonical-root + remote)[..16] —
  deterministic across restarts, distinct per workspace.
- `same_workspace()` compares canonical roots only.
- Backwards compatible: old fields populated; new fields `Option` with
  serde defaults; existing `provenance.rs` fixtures updated.

Supports indexing (identity JSON in `repo_indexes`), cache
invalidation (revision = working-tree hash), facts/impact/history
association (canonical key everywhere). Workspace isolation: different
canonical roots never share an identity.

Tests: canonical stability across `..` inputs, distinct roots differ,
stability across calls, legacy fields populated. The missing-path lexical
fallback mirrors the storage-key normalisation (absolute roots stay
absolute), and `git remote -v` parsing skips blank/malformed lines instead
of aborting the scan.

## 5. File Intelligence

`crates/parsers/src/intelligence/file_classify.rs` (new):

- `FileRecord`: stable id `file::<rel>`, path, language, size, SHA-256
  hash, line count, sorted classes, `parser_supported` + limitation,
  `last_indexed`. Contents never stored.
- `FileClass`: source/test/generated/config/documentation/build/ci
  (multi-label, deterministic).
- `record_for_content()`: hash + count + classify from an already-read
  buffer (caller owns size-gating).
- Generated detection: path segments (`target/`, `dist/`, `*.min.js`,
  `generated`) + content markers (`@generated`, `DO NOT EDIT`).
- Test/config/doc/build/ci heuristics are path/name-based; symbol-level
  `is_test` refines them downstream.

Change detection uses content hashes throughout (init digests,
`diff_digests`, freshness).

## 6. Symbol Intelligence

No new symbol extractor. Existing tree-sitter platform remains the
only symbol source (Rust, Python, JS, TS, Go). P6 guarantees:

- Deterministic IDs (`sym::<rel>::name_kind@line`) — unchanged files
  yield byte-identical records (proven by reindex test).
- File, location, language, kind, visibility flow through unchanged.
- Unsupported languages (C, C++, shell, config): file-level
  intelligence preserved; **no symbols invented** (tested: C file
  contributes zero symbols).
- Parser limitations explicitly exposed (`parser_limitation()` +
  `workspace_context.supported_languages`).

Provenance vocabulary (`Verified`/`Heuristic`/`Unknown` in impact;
`ProvenanceType` in facts) unchanged; P6 adds no `INFERRED` claims.

## 7. Dependency Graph

No graph DB. The existing `FactStore` + impact BFS **is** the graph:

- Edge kinds reused: `Calls`, `Imports`, `References`, `DependsOn`
  (projected into module space for calls), `Documents`, `Configures`.
- Provenance preserved: verified (AST) vs heuristic (name) with
  confidence decay `0.95/0.55 × 0.85^hops`.
- Queries supported: neighbors, dependencies/dependents (direction
  filter), references, callers/callees (call edges), `tests_for`
  (`TestFact.tested` + module containment).
- Deletion cleanup: model rebuilt from current file list; removed paths
  disappear with no orphaned edges (tested: every `Calls` edge resolves
  after deletion).
- Deterministic traversal (sorted adjacency, BTree maps), bounded
  (`max_nodes` default 1000, `max_results`, depth ≤ 5).

## 8. Incremental Indexing

`crates/indexer/src/init/engineering.rs`:

- `FileDiff { added, deleted, modified, unchanged }` — pure
  `diff_digests(prev, curr)` over digest maps; sorted; `is_clean()`,
  `needs_reparse()`, `changed_count()`.
- Parse-layer incrementality already exists (content-addressed parse
  cache: only changed files reparsed); P6 documents + surfaces it
  (`reindex.incremental`, `needs_reparse()`).
- Unchanged files preserved via deterministic IDs (byte-identical).
- Deleted files leave no orphans (rebuild from live file list).
- `scan_current_digests()` helper for freshness scans (same skip-list
  + size gate as init; tracks parsed + manifest files so asset churn
  never marks the index stale).
- Relation to existing `compute_facts_diff`: `diff_digests` is its pure
  kernel (documented in code); no traversal logic duplicated.

Tests: added/deleted/modified/unchanged, hash change, determinism,
orphan-free deletion, clean-when-identical.

The MCP `reindex` response reuses this exact kernel
(`diff_digests_for_mcp` delegates to `diff_digests`, then applies bounded
presentation truncation: 100 entries per list with exact totals and a
`truncated` flag — never a silent drop). A failed `reindex` preserves the
previous row's counts/revision/timestamps and only moves the status to
FAILED, so a failure never zeroes last-good metadata.

## 9. Impact Analysis

Strengthened, not replaced (`crates/impact-engine`):

- Existing BFS, depth/direction/relationship filters, bounded output,
  deterministic ordering, freshness — unchanged.
- **New additive `risk` field** (`RiskSignal { level, indicators[],
  blast_radius }`) on every `ImpactResult` (including `NotFound` with a
  LOW default for shape stability).
- `risk.rs`: deterministic `assess_risk(RiskInput)` — HIGH (persistence,
  migrations, public MCP API, trust boundaries, filesystem publishing,
  concurrency/fencing), MEDIUM (many dependents ≥10, highly shared ≥5
  direct, central public utility, public API), LOW (isolated leaf).
  At most 8 indicators, sorted; blast-radius summary is a bounded
  template, not prose.
- MCP `impact_analyze` needs no param change; responses now include
  `risk` automatically. Depth limits, truncation metadata, and
  provenance summaries unchanged.

Tests: direct/transitive, depth 0/1/2/5 + rejection at 99, bounded
`max_results`, deterministic repeat, risk HIGH/MEDIUM/LOW + ordering.

## 10. Repository Health

Two layers:

1. **`impact::health::analyze_health(store, stale, limit)`** (new,
   pure, no I/O): `HealthFinding { type, severity, evidence, location?,
   confidence }` for CYCLE (bounded DFS, ≤8 cycles), HIGH_FANOUT/FANIN
   (≥20), ORPHAN (edgeless source modules; docs/config excluded),
   UNRESOLVED_REFERENCE (validation `broken_index` count), STALE_INDEX
   (explicit flag), MISSING_TEST_ASSOCIATION (source modules with
   symbols but no linked test), LARGE_MODULE (≥100 symbols).
   Deterministic ordering (type → location → evidence), bounded
   (default 50, cap 500). Observations, never bug claims; no scores.

2. **Doctor `engineering_health` + `engineering_languages`** (additive
   checks 8–9 in `crates/mcp-server/src/doctor`): projects the
   persisted store through `analyze_health`. Severity drives outcome:
   Error → fail, Warning → warn, Info-only → pass (tiny healthy
   workspaces stay green). Languages check discloses recorded language
   surface. Checks never mutate; absent `facts.json` skips (existing
   `facts` check covers absence).

MCP `repository_health` delegates to doctor, so findings flow through
with no handler change.

## 11. Persistence

- **Facts** stay in per-project `.codebro/facts.json` (immutable model +
  `file_digests` + `generation_repo_state`). No schema change; new
  consumers only read.
- **SQLite v6 → v7** (`crates/context-runtime/src/db.rs`): one additive
  table `repo_indexes` (workspace_root PK, identity JSON, status,
  indexed_at, revision, file/symbol/edge/stale counts, updated_at) +
  status index. `IF NOT EXISTS` everywhere; crash-safe resume;
  restart-safe; repeatable; no backfill (absent = UNKNOWN).
- **Runtime** (`repo_index.rs`): `get_repo_index` (absent → in-memory
  UNKNOWN, never fabricated READY), `upsert_repo_index` (canonical
  key, bounded + redacted identity JSON ≤4 KiB with truncation marker).
- Canonical/derived documented in migration comments + this doc.
  Repository contents never duplicated into SQLite.

Migration tests: fresh v7, v1→v7 chain, v6→v7 preserving tasks,
interrupted-v7 resume, idempotent re-run.

## 12. MCP Surface

24 tools, unchanged count. No CRUD for files/symbols/edges/rows.

| Tool | P6 strengthening |
|---|---|
| `workspace_context` | + `repository_identity`, `index_freshness` (live + persisted), `architecture.summary`, `supported_languages` (parsed vs file-level + limitation) |
| `engineering_facts` | Unchanged shape; freshness + bounded deterministic ranking already P6-conformant |
| `impact_analyze` | + `risk` (automatic; no new params) |
| `repository_health` | + `engineering_health` / `engineering_languages` via doctor |
| `reindex` | + `incremental { added, deleted, modified, unchanged_count }`, `index_status`, persisted `repo_indexes` upsert, `index_completed`/`index_failed` history events |
| `context` | No change needed: already consumes facts/decisions/memory/evidence/impact guidance through bounded retrieve→rank→resolve→compress→bound |

All responses bounded (facts ≤50, impact `max_results`, findings ≤50,
indicators ≤8, lists truncated at 100, identity JSON ≤4 KiB).
Deterministic ordering (sorted digests, edges, findings, indicators).
No storage internals exposed.

## 13. Context Integration

`engineering_context::compose` already implements the required
discipline: repository orientation + fact counts, keyword-ranked facts
(≤8 keywords, deduped, capped), decisions, memory excerpts,
evidence-journal status, freshness note, impact suggestions (names
only, no traversal), validation pointers, durable record excerpts —
each provenance-tagged (`verified`/`recorded`/`observed`/`derived`).
P6 flows through it without dumping the repository, graph, history, or
fact store. No composer change was needed; this doc records the
conformance.

## 14. History Integration

New additive `HistoryKind`s: `index_completed`, `index_failed`,
`impact_analyzed`, `health_analyzed`, `repository_discovered`
(importance 55: structural evidence, below decisions/validations).

Policy (enforced): only `reindex` writes history (`index_completed` on
success with file/symbol/edge + diff summary; `index_failed` on error
with bounded message). `impact_analyze` and `repository_health` are
read-only and write nothing (avoids recursion + bloat). No per-file or
per-operation events. History stays append-only.

## 15. Learning Boundary

`learning::kind_group` explicitly maps all five P6 kinds to `Ignored`
(documented: routine indexing must never become a learning candidate —
"index completed → hypothesis" is the canonical anti-example).
Only repeated engineering *outcomes* (task completions, validation
failures) feed P3 through existing rules. No trust bypass; accepted
hypotheses still persist as `AI_INFERRED` only.

## 16. Skills Boundary

P4 owns lifecycle (candidate → validate → approve → version → rollback
→ health); OpenCode executes `SKILL.md` natively. P6 does not execute,
modify, publish, or approve skills. It exposes repository facts relevant
to applicability (languages, workspace shape, test presence) through
existing read paths. No skill tables touched.

## 17. Task Boundary

P5 owns durable task state (lifecycle, checkpoints, leases, fencing,
optimistic concurrency, idempotency). P6 provides engineering
information *about* tasks (impact, health, freshness) but never drives
transitions, schedules, queues, or executes work. Task execution stays
with OpenCode. No new concurrency architecture; `reindex` reuses the
existing workspace mutation lock.

## 18. Security

P0–P5 rules preserved and extended:

- Workspace isolation: canonical keys on every read/write (`RepoIdentity`,
  `repo_indexes`, tasks, history); `same_workspace()` gate; isolation
  tests (A cannot read B) for indexes, tasks, health, MCP.
- Path safety: no `..` traversal (lexical normalisation), no symlink
  escape (`follow_links(false)` + canonical roots), no arbitrary writes
  (health/context/impact are read-only; `reindex` writes only
  `.codebro/facts.json` + its own SQLite row).
- Redaction: identity JSON is redacted through the central
  `redact_secrets_public` authority (key/value secrets, bearer/PAT shapes,
  URL-embedded credentials) and bounded before storage; history summaries
  carry counts only. Secrets never enter the index.
- No authorization through intelligence: risk/health are signals for
  OpenCode to reason over, never access grants. No cross-workspace graph
  reads; no cross-task leakage (task history needs its task).

## 19. Concurrency

No new architecture. `reindex` holds the existing per-workspace mutation
lock for the whole handler (as before); read paths (`workspace_context`,
`engineering_facts`, `impact_analyze`, `repository_health`, `context`)
take no mutation lock. SQLite WAL + busy timeout + single-writer
assumption unchanged. Indexing has no background component.

## 20. Tests

| Area | Coverage |
|---|---|
| Repository identity | canonical stability, distinctness, restart stability, legacy fields (`core`) |
| File indexing | add/modify/delete/unchanged/hash, determinism, orphan-free deletion |
| Symbols | deterministic IDs across reindex, parent linkage via existing pipeline, C file yields zero symbols + limitation string |
| Graph | call edges, callers via impact, deletion cleanup (all `Calls` resolve), deterministic repeat |
| Incremental | pure diff unit tests + `reindex.incremental` E2E + existing `compute_facts_diff` suite |
| Impact | direct/transitive, depth 0/1/2/5, depth-99 rejection, bounded `max_results`, risk HIGH/MEDIUM/LOW + ordering |
| Health | stale present/absent, cycle, orphan, deterministic + bounded |
| Isolation | repo-index A/B, task cross-workspace refusal, MCP `repository_health` per-workspace |
| Migration | fresh v7, v1→v7 chain, v6→v7 preserving tasks, interrupted-v7 resume, idempotence |
| MCP | 24-tool count, no CRUD names, bounded responses, P6 fields present |
| E2E (real binary) | temp repo → `serve` → reindex → query → modify → reindex (diff) → impact (risk) → health → restart → persisted identity + new symbol; hermetic `CODEBRO_STATE_DIR`; `~/.codebro` untouched |

Suite: **1314 passed / 0 failed** (`cargo test --workspace`).
Clippy: clean (`-D warnings`, all targets/features). Fmt: clean.
Dependency direction: `scripts/check_workspace_deps.sh` OK.
`~/.codebro` pollution: NO (hermetic state dirs; home `state.db`
mtime predates the session; repo `.codebro` mtimes predate the session).

## 21. Limitations

- File digests cover parsed source files + manifests (the init
  pipeline's discovery set). C/C++/shell/config files get file-level
  intelligence but do not yet enter `file_digests`; their changes are
  caught by git-state freshness, not by digest diff, in non-git
  workspaces. Extending digest coverage would change `facts.json`
  content and was deferred to preserve P0–P5 stability.
- No C/C++/shell symbol extraction (by design — never invent symbols).
- Cycle detection is module-space DFS bounded at 8 cycles; huge graphs
  truncate deterministically (reported via finding counts, not hidden).
- `repo_indexes` is written only by `reindex` (not by bare `codebro
  init` CLI runs); CLI-only indexes read as UNKNOWN until the next
  MCP reindex — never as fabricated READY.
- Risk signals are heuristic name/path/count rules with stated
  evidence; they are triage aids, not security proofs.
- No embeddings, no vector search, no semantic understanding beyond
  deterministic lexical matching (by design).
- Live freshness needs git: outside git repositories the working-tree
  hash is unavailable, so freshness reports `unknown` (never fabricated
  `fresh` or `stale`). The digest-based `engineering::compute_freshness`
  helper remains available as library API for digest-driven checks.
- Revision-signal change (post-audit): the working-tree hash now covers
  untracked file *contents* (512 KiB gate, path+size above it) and
  excludes derived `.codebro/` output. Git workspaces indexed before this
  change report `stale` exactly once (hash vocabulary changed); the next
  `reindex` heals it permanently. Previously, editing an untracked file
  read as `fresh` (missed signal) and any un-ignored `.codebro/` output
  read as `stale` immediately after indexing (self-inflicted staleness).

## 22. Future Work

- Digest coverage for file-level-only source languages (additive
  `file_digests` entries without symbol/module churn).
- `BOUNDARY_CROSSING` finding activation once package-dependency
  endpoints are projected (type exists; emission deferred for lack of
  a proven edge source).
- `impact_analyzed` / `health_analyzed` history writes behind an
  explicit opt-in (currently read-only by anti-recursion policy).
- Optional `engineering_files` JSON sidecar for file inventory caching
  (only if measured re-scan cost justifies it; SQLite stays
  metadata-only).
- P7 is explicitly out of scope for this phase.

---

## P5 Task ↔ Skill Debt (P6 resolution)

**Decision: B (safe read-time resolution) with A (explicit documentation).**

`skill_refs` (opaque usage association on tasks) and task-scoped skill
rows (P4 lifecycle state keyed by `task_id`) are two different
concepts; the code now documents this at `resolve_task_skill_refs`.
Writes are unchanged (bounded opaque strings, never dereferenced, never
mutating skill health). Reads gain `resolve_task_skill_refs()`: exact
`skill_id`, else exact `name`, against workspace-visible skills (same
canonical root or global scope); unmatched refs returned opaque and
sorted; cross-workspace resolution refused (tasks are workspace-bound).
No complex join system, no execution adjacency.
