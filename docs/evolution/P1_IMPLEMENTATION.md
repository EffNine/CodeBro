# P1 Implementation — User Fingerprint + Intent

Date: 2026-09-06. Status: **implemented, tested, verified** (329 mcp-server
lib tests + 75 context-runtime tests green, clippy/fmt/dep-script clean).
Scope: `docs/evolution/PHASE4_PLAN.md` P1 row, built on the P0 foundation as
reviewed in `docs/evolution/P0_POST_IMPLEMENTATION_REVIEW.md` (verdict: PASS
WITH CHANGES — P1 approved).

P1 makes CodeBro begin understanding who the user is as a collaborator, how
they prefer to work, and what they are currently trying to achieve — with no
reasoning engine, no agent loop, no inference, and no vector search. OpenCode
decides when context is needed; CodeBro decides what context is relevant;
CodeBro preserves explicitly-confirmed knowledge.

## 1. Architecture

Extension over replacement. P0's `ContextRecord` table already had the
fingerprint/intent shape (`Preference`/`Style`/`Taste`/`Intent` kinds,
`namespace`, `original_text`/`language`, six-authority provenance, three-way
scope, supersede chains, FTS5). P1 adds the missing trust and resolution
pieces around it:

```text
context-runtime (new modules, same crate, still → core only)
├── types.rs      + task_id, extra_json, authority_rank,
│                   lifecycle_for_authority, tightened validate_record
├── workspace.rs  canonical_workspace_key (lexical + best-effort symlink)
├── fingerprint.rs lanes, scope_rank, applies, winner_order,
│                   resolve_context (per-namespace winners + intents)
├── intent.rs     IntentStatus/IntentPriority/IntentMetadata over extra_json,
│                   validate_transition, partition_intents
├── db.rs         SCHEMA_VERSION 2, stepwise migrate (v1 batch → v1+v2 steps)
└── store.rs      normalization, evidence existence check, task-aware
                    search/count, retire_record

mcp-server (thin adapters, composition stays in engineering_context.rs)
├── remember / forget tools (semantic writes, not table exposure)
├── context tool: + task_id arg, resolution-based records section,
│                  keyword+importance merged fetch
└── excerpts: + kind, scope, task_id, decoded intent block
```

No schema redesign, no second lifecycle, no new store. The five P0 review
gates (§13: evidence existence, caller-principal rule, hierarchy resolution,
task identity, canonicalization) all land here.

## 2. Data model

Two additive nullable columns (`task_id TEXT`, `extra_json TEXT`) plus one
index (`idx_records_task`), migrated stepwise v1→v2:

- **v1 databases upgrade in place**, rows preserved with `NULL` new columns
  (read back as `None`), FTS index intact, no quarantine (migration ≠
  corruption). Tested: `v1_database_upgrades_to_v2_preserving_rows`.
- **Partial v2 resumes**: column-presence probes before `ALTER TABLE`, so a
  crash mid-migration never fails on its own half-applied column. Tested.
- **Fresh databases** get both columns. Tested.

`extra_json` is the deliberate anti-column sprawl mechanism: one schemaless
JSON-object column (≤2048 chars, must parse as an object) instead of dozens
of rigid preference columns. Namespaces (`fp.*`, `intent.*` by convention,
unenforced) keep it extensible. Intent owns `rationale`/`priority`/
`intent_status` keys via `IntentMetadata`; future phases add keys, not
columns.

## 3. Authority rules (trust / provenance)

Provenance is structural, enforced at the lowest layer that can enforce it:

1. **Evidence-id existence** (store, `check_evidence`): every cited id must
   parse as an integer event row id, the event must exist, and for
   workspace-scoped records the event must belong to the same canonical
   workspace. Global records check existence only. Runs inside the write
   transaction (`put_record`, `supersede_record`/`retire_record` via
   `supersede_impl`), so supersede-with-forged-evidence fails atomically.
2. **Caller-principal rule for UserConfirmed** (`remember` tool):
   `authority=user_confirmed` is refused unless `user_confirmed=true` is
   also set. The flag is OpenCode's explicit speech act — "the user stated
   or approved this" — which is the only trusted basis CodeBro has behind
   MCP. Forgery would require the agent to lie about the user, which is
   outside CodeBro's threat model (same standing as `record_memory`
   key/value trust). No fake PKI, no session tokens.
3. **Lifecycle floor** (`lifecycle_for_authority`): UserConfirmed→Confirmed,
   AiInferred→Inferred, everything else→Observed. `LifecycleStage` and
   `Authority` stay separate fields (P0 rows predate the rule; the store
   accepts any stage) — the semantic write layer assigns both together.
   Documents the P0 duplication finding without a breaking migration.
4. **Reference-only reservation enforced**: `Fact`/`Decision`/`Skill`
   records without `related_ids` are refused in `validate_record` (was
   comment-only in P0). They mirror the JSON stores, never duplicate them.
5. **No downgrade path**: nothing rewrites authority in place; promotion
   goes through supersede with the audit trail. `forget` defaults to
   reversible reject (negative knowledge preserved); hard remove needs
   `permanent=true` + `confirm=true` (junk cleanup only).

## 4. Namespace resolution

Deterministic per-(kind, namespace) winners for a (workspace, task)
viewpoint (`fingerprint::resolve_context`):

```text
authority rank  user_confirmed(100) > project_derived(50) > system_derived(40)
                > imported(30) > observed(20) > ai_inferred(10)
  → scope specificity   task(3) > project(2) > global(1)
    → effective (decayed) confidence → updated_at → id (total order)
```

Authority outranks specificity deliberately: a confirmed global is never
silently overridden by an inferred project guess; within equal authority
(the normal case) the more specific scope wins. Task rows are invisible
without a matching `task_id`. Losers are reported as `suppressed` (audit),
never deleted. Ordering is total and input-order-independent (tested).

Lanes: fingerprint kinds (`Preference/Style/Taste/Constraint/Principle/
Pattern`) resolve per-namespace; `Intent` resolves to the actionable set
(active rows with actionable decoded status; malformed `extra_json` is
counted, never defaulted); `Experience/Other` pass through for P3.

## 5. Fingerprint semantics

Structured semantic records, one row per (kind, namespace, scope) —
communication, engineering, product/design, and working-style dimensions
are all just namespaces, no rigid columns. Semantic content, not
transcription (`content` holds the interpretation, `original_text` + 
`language` keep the verbatim evidence, e.g. Manglish source → English
canonical statement). Updates go through `supersedes` (old row →
`Superseded`, both queryable); blind second writes to an owned namespace
are refused naming the incumbent. Negative knowledge (`Rejected` via
`forget`) stays for future avoidance.

P1 persists **only explicit confirmation** (`USER_CONFIRMED` needs the
flag) and **agent observations with minted evidence** (`observation`
mints an `agent_observation` event, cited automatically). There is no
conversation→infer→persist path; `AiInferred` is writable only with real
evidence ids and stays visibly inferred.

## 6. Intent semantics

`RecordKind::Intent` + `IntentMetadata{rationale?, priority?, intent_status}`
in `extra_json`. Intent status reuses the storage lifecycle instead of
competing with it: active/paused rows are `Active`; completing expires and
cancelling rejects via `retire_record` (old row → `Superseded`, terminal
replacement linked — the full attempt history stays queryable).
`Superseded` intent status requires `supersedes`. Terminal intents refuse
further supersession (`validate_transition`); completing an intent without
`supersedes` auto-retires the single active intent in the namespace, and
errors on ambiguity. Preference/decision/intent stay three kinds linked by
`related_ids`, never one generic memory blob.

## 7. MCP surface (capabilities, not tables)

Two new tools (20 total), no per-table CRUD:

- **`remember`** — semantic persist. Required: `content`, `namespace`.
  Defaults: kind=preference, scope=project, authority follows the confirm
  flag. Intent fields (`rationale/priority/intent_status`) only with
  kind=intent. Fresh writes clash-check the namespace (refuse naming the
  incumbent, except unambiguous terminal-intent auto-retire); replacements
  need `supersedes`. Secrets redacted before storage. Mutating: holds the
  workspace lock.
- **`forget`** — semantic retire. `id` or `namespace` (+task for task
  rows); `confirm=true` required; default reversible reject, `permanent`
  for hard remove. Workspace-confined: project/task rows only from their
  own workspace. Holds the workspace lock.
- **`context`** — additive `task_id` arg; records section now resolves
  (fetch 100 importance-ordered + 100 keyword matches merged by id →
  `resolve_context` → intents + winners + passthrough, capped at 8).
  Excerpts gain `kind`, `scope`, `task_id?`, and decoded `intent?`
  (status/priority/bounded rationale). Server instructions advertise the
  flow: context-with-task_id at task start, remember-only-what-was-
  confirmed, forget-to-retire.

## 8. Retrieval behavior (honest statement)

P0 review noted the architecture doc over-claims ("BM25 blended with the
lexical scorer + authority/decay weighting"). Actual P1 behavior, stated
plainly: keyword queries rank BM25, keyword-less rank importance/recency;
`effective_confidence` (authority-differentiated decay) is computed per hit
and reported, and now *participates* as the third precedence key in
namespace resolution — but there is still no cross-signal score blending
and no semantic search. That remains correct to defer (P3 seam untouched).

## 9. Context composition

`MAX_CONTEXT_RECORDS = 8` unchanged; resolution output (intents first, then
fingerprint winners, then passthrough) fills it. Structural digest (no
task) resolves with task=None: global + project fingerprint and active
intents present at session start. Malformed intents never enter the packet
(counted in `malformed_intents`). Provenance fully visible per excerpt
(authority + kind + scope + decayed confidence + language).

## 10. Security / trust invariants

- USER_CONFIRMED unforgeable by parameter-naming (flag gate).
- Evidence must exist, same-workspace for scoped records (store backstop).
- Task rows invisible without the task; cross-workspace forget refused;
  cross-project reads return nothing (same state.db, strict key match).
- Canonical keys: lexical normalization + best-effort symlink resolution;
  `/repo`, `/repo/`, `/repo/sub/..`, symlinked aliases share one namespace
  (write, query, event, and count paths all normalize).
- Redaction on all free-text write paths; bounded payloads everywhere;
  mutating tools serialize on the workspace lock (remember/forget pinned
  by test); no `~/.codebro` pollution in tests (`CODEBRO_STATE_DIR` /
  explicit state dirs).

## 11. Tests added (all green)

context-runtime 38 → 75: task HistoricalId validation (6), extra_json +
reference-only + lifecycle/rank (4), canonicalization (5), v2 migration
(3: fresh columns, v1 upgrade preserving rows + FTS, partial-step resume),
evidence gates (4: ghost/malformed/foreign-workspace/supersede-atomic),
canonical write/query (1), task visibility both query paths (1),
task_id/extra_json round-trip (1), precise counting (1), fingerprint
resolution (8: specificity, confirmation-beats-inference, namespaces,
task isolation, workspace isolation, global view, determinism, ranks),
intent metadata/transitions/vocab (6).

mcp-server: 12 P1 tool tests (confirmed round-trip, principal gate,
observation minting + fiction refusal + inferred-stays-inferred,
task>project>global, confirmed-beats-inferred, supersede history,
intent active→pause→complete→terminal-refusal, 7-way shape rejection
battery, forget confirm/reject/namespace/permanent/isolation,
workspace isolation + canonical equivalence, boundedness at 20 rows,
mutation-lock serialization).

## 12. Known limitations (not defects)

- `RecordQuery.task_id` + `count_visible_with_task` extend rather than
  replace the P0 task-less variants (kept for compatibility).
- `parse_list` silent-`[]` on corrupt JSON and the `ContextProvenance`
  second vocabulary (P0 review notes) are untouched — not P1 blockers.
- Keyword fetch uses FTS5 AND semantics; merged with the importance fetch
  so recall never narrows to zero.
- No cross-process writer test yet (P0 gap, still open); in-process
  serialization pinned for the new tools.
- `remember` id minting loops on collision (bounded in practice: ids embed
  unix time + slug).

## 13. Explicit non-goals (not built)

Automatic learning, inference engine, embeddings/vector search, skill
creation or self-modification, durable task runtime, personal-assistant
automation, memory import/export, TUI, agent loop — none present. `learn`
remains a P3 design; `Experience` rows are storable but nothing mints them
autonomously.

## 14. P2 dependencies handed off

`task_id` column + task-aware search/count are the identity substrate P2
sessions need; `sessions` table shape still reserved (P0 note stands);
`ContextRetriever` seam untouched for `recall`; event log now carries
`agent_observation` rows learning can consume; `malformed_intents` count
is the corruption-visibility pattern to reuse.
