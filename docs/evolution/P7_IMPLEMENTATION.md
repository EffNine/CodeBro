# CodeBro P7 Implementation — Engineering Decision Support

**Status:** COMPLETE (additive over frozen P0–P6).
**Schema:** v7 (unchanged — no new storage).
**MCP tools:** 25 (one addition: `engineering_brief`; no CRUD).
**Architecture:** CodeBro prepares evidence; OpenCode reasons, plans, executes.
No agent loop, scheduler, daemon, watcher, model calls, or autonomous execution.

---

## 1. Mission

P6 answers "what exists in the repository, how is it connected, what is
affected, and what is unhealthy". P7 answers "for this engineering task,
what evidence and context should OpenCode know before making a decision".

The core P7 capability is a single deterministic pipeline:

```text
TASK + USER/PROJECT CONTEXT + REPO INTELLIGENCE + IMPACT + HEALTH
  + HISTORY + ENGINEERING MEMORY + LEARNING + SKILLS + TASK STATE
       │
       ▼
BOUNDED ENGINEERING BRIEF
       │
       ▼
OpenCode (reasons / plans / executes)
```

P7 is decision SUPPORT, not decision making. The brief contains evidence,
pointers, constraints, risks, and explicit unknowns. It never selects a
solution, never orders skill execution, and never transitions task state.

## 2. Existing Architecture Reused

No second context composer, no second ranking system. P7 reuses:

| Need | Reused mechanism | Location |
|---|---|---|
| Fact ranking | lexical `search` (exact 100 / prefix 80 / substring 60 / path 30 / summary 15) | `mcp/facts.rs` |
| Fact trust/freshness | `compute_fact_trust`, `compute_freshness` | `mcp/facts.rs` |
| Memory ranking/budget | `resolve_for_task` (≤20 entries, 500-token budget, conf ≥ 0.3) | `memory-runtime` |
| Record resolution | caller-supplied excerpts (fingerprint resolver already ran) | `mcp/mod.rs::context_record_excerpts` |
| History ranking/grouping | `recall` (FTS5 + deterministic priors, session-grouped) | `context-runtime/recall.rs` |
| Learning trust | `list_candidates` with `Accepted`/`Rejected` filters | `context-runtime/learning.rs` |
| Skill data | `list_skills` + P6 `resolve_task_skill_refs` | `context-runtime/skills.rs`, `tasks.rs` |
| Task state | `task_resume_snapshot` (read-only) | `context-runtime/tasks.rs` |
| Impact traversal | `analyze` (bounded BFS + P6 risk signal) | `impact-engine` |
| Health findings | `analyze_health` (pure over the store) | `impact-engine/health.rs` |
| Freshness | live `compute_freshness` + v7 `repo_indexes` row | `mcp/facts.rs`, `context-runtime/repo_index.rs` |
| Bounds | per-section `MAX_BRIEF_*` + global `bounded_response` (256 KiB) | `engineering_brief.rs`, `mcp/response_bounds.rs` |
| Excerpt marker | `…[truncated for context budget]` / `…[truncated]` | `engineering_context.rs` |

New code is one module (`crates/mcp-server/src/engineering_brief.rs`:
request type, brief model, `assemble()` pipeline, section projectors),
one MCP handler (`engineering_brief` in `mcp/mod.rs`), and tests.
`mcp-server` depending on all runtime crates is already allowed by
`scripts/check_workspace_deps.sh` (verified: direction OK, no manifest change).

## 3. Engineering Brief Model

`EngineeringBrief` (`engineering_brief.rs`) mirrors the §5 template with
Rust types suited to the existing architecture:

- `task`, `task_id`, `repository` (identity + project id + languages),
  `freshness` (live + persisted), `scope` (workspace key + keywords),
  `architecture` (summary + relevant modules + languages),
  `targets` (discovery + id/kind/name/path + ambiguity candidates),
  `files`, `symbols` (reused `FactRecord`), `dependencies`,
  `impact` (projected traversal + risk), `tests`, `health`, `history`
  (+ totals/truncation), `engineering_memory`, `learning`,
  `negative_knowledge`, `skills`, `constraints`, `decisions`,
  `decision_conflicts`, `risks`, `unknowns`, `records`, `task_state`,
  `bounds`, `notes`.
- There is deliberately no `decision` field: OpenCode decides.
- Every section carries a `category` (§11 vocabulary) and a `provenance`
  (`verified`/`recorded`/`observed`/`derived`, same vocabulary as the
  context packet). Items with a real `Authority` carry it verbatim;
  identity-sourced items carry `source: "project_identity"` instead of an
  invented authority rank.

## 4. Evidence Sources

P7 consumes, never duplicates: P0 records/decisions/evidence, P1
fingerprint/intent/identity/trust, P2 history/sessions/recall, P3 accepted
(+ rejected-as-negative) learning, P4 skill metadata/health, P5 task/
checkpoint/validation state, P6 identity/files/symbols/graph/impact/health/
freshness. Retrieval is one bounded read per source per brief (no N+1:
facts search is in-memory, recall is one FTS query, learning is two
bounded lists, skills one list, task one snapshot, impact one traversal).

## 5. Retrieval Pipeline

`assemble()` follows §4 exactly:

```text
validate scope/targets → keywords → repo/freshness → task snapshot
  → target discovery → files/symbols/dependencies → impact → tests
  → health → history → memory → learning → skills → constraints/decisions
  → risks → records → bound → brief
```

Empty scope is rejected (`task scope is required`), never answered with a
dump. The handler enriches record keywords with a best-effort read-only
task hint (title/description/next-action tokens); the assembler derives a
sibling enrichment for the brief's own scope (title/checkpoint-summary/
next-action tokens). The two sets overlap but are not identical — records
resolve against the broader set; scope keywords stay assembly-internal.

## 6. Ranking

No new ranking layer — the existing P0–P6 rankings were sufficient, so per
§12 none was added. Section-internal order reuses source ranking (facts
score order, memory resolver order, recall order, fingerprint
resolution). Brief-level combination uses stable sorts only (key, name,
id, confidence desc + id). There is no AI-generated relevance score
anywhere in P7.

## 7. Authority / Provenance

- Real `Authority` strings flow verbatim (`user_confirmed`, `ai_inferred`,
  `observed`, …). Nothing is upgraded for relevance.
- Accepted learning → `authority: "ai_inferred"` (never `user_confirmed`).
- Rejected learning → excluded from `learning`; eligible as
  `negative_knowledge` entries when keyword-relevant.
- Candidate/deferred/expired learning never surfaces (`NO_LEARNING`
  unknown instead).
- Superseded/deprecated decisions carry `current: false`, are never
  presented as current, and feed no recommendation.
- Preferences never become constraints: `constraints` takes identity
  constraints (`hard`) plus `constraint`-kind records (`hard` iff
  `user_confirmed`, else `observed`).
- Conflicting decisions (≥2 shared significant title tokens, differing
  currency) surface as `decision_conflicts`; never resolved by guessing.

## 8. Scope / Isolation

- Workspace: `resolve_workspace` + canonical keys on every read; recall
  uses project/task scope only (global opt-in never used by briefs).
- Task: `task_resume_snapshot` enforces store-level isolation; invisible
  tasks (missing or cross-workspace) yield `TASK_NOT_FOUND` — existence
  and content never leak (pinned by unit, MCP, and RPC tests).
- Task history is included only when its task is named (recall task scope).
- Records are caller-resolved excerpts (existing fingerprint pipeline);
  the brief embeds at most 8, sorted by id.

## 9. Freshness

Live (`fresh`/`stale`/`unknown`) + persisted (`READY`/`STALE`/`FAILED`/
`UNKNOWN` + `indexed_at` + revision, last-good preserved on failure).
`stale` ⇒ `STALE_INDEX` unknown + note + risk signal; `FAILED` ⇒
`FAILED_INDEX`; neither ⇒ `UNKNOWN_FRESHNESS`. Stale intelligence is
labelled as last-indexed-state evidence, never current truth. E2E pins the
fresh → stale → fresh cycle across modify/reindex.

## 10. Architecture Integration

Deterministic P6 facts only: identity `architecture_summary` (excerpted)
plus relevant modules (top files by task relevance ∪ keyword-hit
`known_modules`, sorted, ≤8). No LLM essay. File-level-only languages
contribute paths/counts, never symbols; task keywords naming them yield
`UNSUPPORTED_LANGUAGE`.

## 11. Impact Integration

One bounded traversal per brief (`depth` default 1, max 2; explicit or
singly-discovered targets only). Projection keeps target, direct (≤10 +
total), transitive (≤10 + total), nodes visited, truncation flag, and the
P6 risk signal (level + indicators + blast radius). Ambiguous/missing
targets skip traversal with `AMBIGUOUS_TARGET`/`TARGET_NOT_FOUND`.
Deeper graphs belong to `impact_analyze`. Risk stays a signal: it also
feeds the brief-level `risks` list, never a conclusion.

## 12. Health Integration

`analyze_health` over the store, then task-relevance filtering
(`STALE_INDEX` always; otherwise keyword/target location overlap), ≤10,
deterministically ordered. Unrelated findings are excluded by construction.
Warnings/errors additionally feed `risks` with `source: "health"`.

## 13. History Integration

`recall` with the brief keywords (task scope when a task is named),
≤8 excerpts carrying kind/session/stale/task-match provenance. No
transcripts, no raw payloads, no row ids. Empty/unqueryable history yields
`NO_HISTORY` (unavailable, not absent). P6 index kinds stay learning-
ignored; brief reads add no history (no recursion).

## 14. Learning Integration

Accepted candidates with keyword overlap → `learning` (≤5, confidence
desc + id). Rejected with overlap → `negative_knowledge` (≤5, with failed
tasks/validations). P3 trust semantics preserved; brief performs no
detection, evaluation, confirmation, or rejection.

## 15. Skills Integration

Applicability information only: registry actives with language/subsystem
applicability evaluated against repo languages and task keywords
(`applicable` + reason; unscoped skills marked uncertain, non-matching
filtered), plus task-referenced skills via P6 read-time resolution
(resolved + opaque-unresolved, `origin: "task_ref"`), sorted applicable-
first + name, ≤8. No execution, approval, publication, or modification;
the brief never says "execute skill X" (pinned by test).

## 16. Task Integration

Read-only `task_resume_snapshot`: id/title/status/priority/version/stale,
checkpoint summary + next action, validation state, ≤5 recent events,
intent note. No transitions, no checkpoints, no validation writes; the
handler takes no mutation lock (pinned by read-only + concurrency tests).
Task text (title/description/next-action) deterministically enriches
keyword scope. Failed tasks/validations feed `negative_knowledge`.

## 17. MCP

One semantic capability, `engineering_brief` (tool 25):

- Input: `task?`, `task_id?`, `target_path?`, `target_symbol?`,
  `target_module?`, `keywords?`, `depth?`, `workspace_root?` (canonical
  identifiers only; ≥1 scoping signal required).
- Output: bounded deterministic `EngineeringBrief` JSON through the
  standard 256 KiB `bounded_response` envelope.
- No CRUD getters (`get_file_context`, `get_symbol_context`, …) were
  added (pinned by the tools/list test).
- Errors: `invalid_params` for empty scope / blank or traversal-shaped
  targets / absurd depth; unknowns (not errors) for everything unresolvable.

## 18. Bounds

Explicit caps: keywords 16, files 10, symbols 10, dependencies 10,
impact 10+10, tests 10, health 10, history 8, memory 5, learning 5,
negative 5, skills 8, constraints 10, decisions 8, conflicts 5, risks 8,
records 8, ambiguity candidates 5; excerpts 240 chars, values 500 chars;
depth ≤ 2; global 256 KiB envelope with `truncated_sections` reporting.
Truncation is head-based with totals, deterministic.

## 19. Security

P0–P6 guarantees preserved: canonical workspace keys, task isolation,
provenance on every section, redaction inherited from write-time
authorities (no raw payloads/rows/ids beyond canonical opaque
identifiers), bounded output, traversal denial on targets, no
authorization derived from intelligence. Adversarial coverage: cross-
workspace/task invisibility (unit + MCP + RPC), traversal rejection,
ambiguous-name refusal, opaque unresolved skill refs. The
post-implementation audit closed three redaction gaps at write seams the
brief newly surfaces: skill `description`/`purpose` are now redacted at
`skill propose` (plus defense-in-depth in the brief's skills projection),
task `skill_refs` are now redacted at `create_task`/
`set_task_skill_refs` (plus projection defense), and every `update_identity`
free-text field is now redacted (plus projection defense for legacy rows) —
the same `redact_secrets_public` authority every other free-text write seam
uses.

## 20. Concurrency

No new architecture. Assembly takes no mutation lock and performs only
short read transactions; it runs concurrently with `reindex`, task
mutation, and skill mutation (pinned by test), and concurrent briefs
agree byte-for-byte on stable state. Cross-process writers keep the
documented single-writer assumption. No caching was added (retrieval is
already in-memory/bounded; no evidence justified a cache).

## 21. Tests

| Area | Coverage |
|---|---|
| Unit (21, `engineering_brief::tests`) | assembly, empty-scope rejection, target validation, discovery (explicit/discovered/ambiguous/none), unknowns taxonomy, freshness variants incl. FAILED preservation, unsupported language, deleted/explicit file targets, empty-store honesty, learning accepted/rejected lifecycles, determinism ×2 (repeat + keyword order), cross-workspace-task invisibility, bounds, cycle termination, huge history/memory bounds, depth clamp |
| MCP (14, `mcp::tests::brief_*`) | empty scope, bad targets, empty-workspace brief, task read-only state, cross-workspace unknown, confirmed constraints vs preferences, superseded currency, conflicts, failed-task negative knowledge, history excerpts bound, skill-ref bounds + no-execution language, determinism, 8-way concurrency + writelessness, concurrency with reindex/task/skill mutation |
| Isolation | workspace A/B (facts/memory/tasks/brief), task-id cross-workspace (unit + MCP + RPC) |
| Determinism | repeat assembly, reordered keywords, concurrent agreement, restart byte-equality (RPC) |
| Bounds | per-section caps, 256 KiB envelope assertion, history/memory stress |
| Real binary E2E (3, `tests/p7_brief_e2e.rs`) | task → index → brief → modify → stale brief → reindex → changed impact/freshness → restart determinism; cross-workspace isolation; malformed inputs + 25-tool inventory with no CRUD getters |
| Adversarial (16, `tests/p7_brief_adversarial.rs`, post-audit) | secret redaction at every new brief read seam (skill descriptions, task skill refs, task titles, identity free-text), authority-conflict preservation, task-scoped record isolation, ambiguous-symbol refusal, depth bounds, failed-reindex FAILED_INDEX honesty, taskless no-invention, cross-domain no-decision-language, repository-identity stability across restart/repos, corrupt state.db degradation, 300-symbol store bounding, output hygiene (no row ids/payloads) |
| Regression | full P0–P6 suite green; P6 24-tool test updated to 25 with `engineering_brief` required |

Suite: **1360 passed / 0 failed** (1322 baseline + 38 new) at implementation
time. The post-implementation audit
(`P7_POST_IMPLEMENTATION_AUDIT.md`) fixed two secret-redaction gaps (skill
descriptions, task skill refs) and added 15 adversarial regression tests
(`tests/p7_brief_adversarial.rs`); suite total is now 1375. Clippy
`-D warnings` clean (all targets/features). `cargo fmt --check` clean.
`scripts/check_workspace_deps.sh` OK. `~/.codebro` untouched (home
`state.db` mtime predates the session; all suite state hermetic).

## 22. Limitations

- Depth ≤ 2: deep/cyclic graphs are safe (tested) but summarized; full
  traversal belongs to `impact_analyze`.
- Keyword discovery is lexical (inherits `engineering_facts` semantics):
  vague task text yields `NO_TARGET`/`AMBIGUOUS_TARGET` honestly rather
  than a guessed traversal.
- Tests combine impact linkage with module containment; symbol-level
  linkage beyond the impact result is not re-derived (stated in code).
- Recall needs query tokens: keyword-less requests skip history with
  `NO_HISTORY` rather than dumping sessions.
- `repo_indexes` is written by MCP `reindex` only (P6 limitation carried
  forward); CLI-only indexes read as `UNKNOWN` persisted status.
- No embeddings, no vector search, no model calls (by design).

## 23. Future Work

- Digest coverage for file-level-only languages (P6 debt, unchanged).
- `BOUNDARY_CROSSING` activation (P6 debt, unchanged).
- Opt-in `impact_analyzed`/`health_analyzed` history writes (P6 debt).
- Engineering-files sidecar caching if re-scan cost justifies it (P6 debt).
- P8 is explicitly out of scope.
