# Phase 4 — Implementation Plan

Date: 2026-09-06. Every increment is independently testable; existing behavior preserved throughout.
Approved scope: **P0 now**. P1–P6 designed for, not implemented until separately approved.

| | Increment | Deliverable | Tests |
|---|---|---|---|
| **P0** | Persistent Context Foundation (IN PROGRESS) | `context-runtime` crate: `~/.codebro/state.db` bootstrap (WAL, user_version migrations, quarantine-on-corruption); tables context_records/events/sessions + FTS5; domain types (record_type/authority/scope/status/lifecycle) + confidence decay; lifecycle transitions + supersede/expire sweep; `context` MCP tool wiring existing `engineering_context.rs`; extensible retrieval trait | migration up/forward, corruption quarantine, authority gates, lifecycle transitions, supersede/expire, FTS5 sync, restart persistence, `context` composition + bounds + no-files-created, isolation test (cannot touch facts/memory) |
| **P1** | User Fingerprint + Intent — **IMPLEMENTED 2026-09-06** (`docs/evolution/P1_IMPLEMENTATION.md`) | `remember`/`forget` tools; preference/intent records with language tagging (canonical vs original); hierarchy resolution task > project > global; USER_CONFIRMED vs AI_INFERRED semantics | semantic upsert, language tagging, evidence-required inference, confirmation supersedes, project isolation, redaction, reversibility |
| **P2** | Session History + Recall — **IMPLEMENTED 2026-09-06** (`docs/evolution/P2_IMPLEMENTATION.md`) | Persistent sessions (opaque ids, active/completed/abandoned, stale-not-completed, restart-safe resume); append-only history reusing EventRecord + closed 11-kind taxonomy; passive capture in handlers (remember/forget/apply/test/build); `recall` tool (FTS5 OR-candidates + canonical filtering + deterministic rank + session grouping + bounded excerpts) | lifecycle determinism, cross-session search, cross-store isolation (FTS-bypass proof), provenance filters, budget bounds |
| **P3** | Learning Engine — **IMPLEMENTED 2026-09-07** (`docs/evolution/P3_IMPLEMENTATION.md`) | Deterministic event → candidate formation (token-pair clustering, ≥3 support, chatter never mined, sensitive topics refused) → outcome-aware evaluation (support/contradict lists, bounded confidence, contested ⇒ defer, majority-against ⇒ reject, global bar) → `AI_INFERRED` persistence (existing records table, evidence-bound, decaying/expiring) → `learn` tool (run/propose/list/get/evaluate/confirm/reject); confirm-gated USER_CONFIRMED promotion, rejection as negative knowledge | hypothesis from repeated outcomes, no-auto-promotion invariant, forged-confirmation refusal, cross-workspace/scope isolation, idempotent reprocessing, concurrent passes converge, failure isolation, context eligibility without composer changes |
| **P4** | Skill Evolution — **IMPLEMENTED 2026-09-07** (`docs/evolution/P4_IMPLEMENTATION.md`, audit in `docs/evolution/P4_POST_IMPLEMENTATION_AUDIT.md`) | Skill metadata tables; validation (agentskills.io lint + threat scan); versioned publication with atomic+symlink-safe writes; `skill` tool; layered trust gates (user_confirmed approval, evidence-backed candidates, stale-version anchors) | validation rejections, checkpoint/rollback, publish idempotence, external-edit refusal |
| **P5** | Durable Task Runtime — **IMPLEMENTED 2026-09-07** (`docs/evolution/P5_IMPLEMENTATION.md`) | `tasks` + `task_checkpoints` tables (schema v6) + `task` tool; strict state machine with validated transitions and a completion gate; immutable atomic checkpoints; worker leases with fencing; bounded resume snapshots; P2/P3 integration (task events are history + learning evidence); no executor, no scheduler | state machine transitions, stale-worker fencing, snapshot restore, bounded refs, workspace isolation, restart/interruption E2E |
| **P6** | Engineering Automation (backlog) | CLI `run` verbs + OS-cron samples (reindex/health/dependency reports) | deferred |

## P1 actual state (2026-09-06 — implemented as planned, plus review gates)

Delivered exactly the P1 row above, with the five P0-review gates as the
load-bearing core (evidence-id existence, caller-principal rule, per-
namespace task > project > global resolution, task-scope identity rule,
workspace canonicalization). Two design refinements vs the plan:

1. **Intent status reuses the storage lifecycle** (`completed`→expired,
   `cancelled`→rejected via new `retire_record`) instead of a competing
   status machine; pending/approved vocabulary deferred — `active/paused/
   completed/cancelled/superseded` covers P1.
2. **Authority outranks specificity** in conflict resolution (confirmed
   global beats inferred project); specificity decides within equal
   authority. The plan's "task > project > global" holds for the common
   equal-authority case and is tested as such.

No schema redesign was needed (two additive nullable columns, stepwise
v1→v2 migration with upgrade + resume tests). Retrieval honesty note: BM25
/ importance ordering stands; authority/decay now participate as
resolution precedence keys, not blended scores — the PHASE3 "blended"
phrasing remains aspirational and is owned by P3. Full detail:
`docs/evolution/P1_IMPLEMENTATION.md`.

## P0 detailed steps

1. Workspace: add `crates/context-runtime` (deps: codebro-core, rusqlite 0.31 bundled, serde, serde_json, uuid or slug-free id scheme, tempfile dev). Wire into workspace members + dependency script allowance (context-runtime → core only).
2. Domain: `Authority`, `RecordKind`, `RecordScope`, `RecordStatus`, `LifecycleStage`, `ContextRecord`, `Event`, `Session` types; decay/trust fn.
3. Store: open/create state dir + db; WAL; `user_version` migration runner; quarantine on corrupt open; insert/update/supersede/expire-sweep/list/query; FTS5 row sync in same tx; bounded payload helper.
4. Retrieval: `ContextQuery` (scope, project, kinds, status, limit) + deterministic ranking; trait seam.
5. MCP: `context` tool — extended `engineering_context` packet + records section; provenance tags; bounded via `response_bounds`; optional `task` (absent = structural digest); per-workspace scoping honored.
6. Tests: per-crate unit + mcp integration (tool present, composition bounds, workspace scoping, no writes to `.codebro/*.json`), restart persistence, migration 1→current, corrupt-db quarantine, two-writer serialization.
7. Docs: AGENTS.md tool table, CHANGELOG (unreleased → v1.3 additive), docs/evolution records, MCP_API_V1 additive note.
8. Quality bar: full `cargo test`, `cargo clippy`, `scripts/check_workspace_deps.sh`.

## Standing constraints

- Never modify existing tool shapes or `.codebro` JSON semantics.
- No embeddings/vector/network/external services; no new agent loops; no executor in P0.
- All filesystem tests via `tempfile::tempdir()`; `CODEBRO_STATE_DIR` override for hermetic tests.
