# CodeBro MCP API v1

Status: **FROZEN for v1.0** (Phase 9 of [V1_ROADMAP.md](V1_ROADMAP.md)).
This document defines the stable, versioned MCP contract. After v1.0,
additions are additive; removals, renames, and semantic changes require a
major version bump plus a migration path.

- Transport: stdio (`rmcp`), JSON-RPC 2.0, protocol version `2024-11-05`.
- Server name: `codebro` (P8: initialize reports `serverInfo: codebro/<crate version>`); binary: `codebro serve --root <workspace>`.
- Tool arguments are passed as JSON strings and deserialized at runtime;
  every schema below is the authoritative shape.
- stdout hygiene (P8): stdout carries ONLY JSON-RPC when `codebro serve`
  runs. tracing and indexer reports write to stderr. One stderr
  observability line per tool call (client identity, tool, duration,
  status, response bytes — identity-only, secret-redacted, never
  persisted, never arguments/brief content).
- Client contract (P8, `docs/evolution/P8_IMPLEMENTATION.md`;
  P9 outcome reporting, `docs/evolution/P9_IMPLEMENTATION.md`): orient
  (`workspace_context`/`context`) → primary evidence
  (`engineering_brief`) → optional targeted follow-up
  (`engineering_facts`/`impact_analyze`/`recall`/`engineering_memory`/
  `repository_health`) → OpenCode reasons/codes/executes with its own
  tools → explicit persistence
  (`remember`/`record_memory`/`task`/`learn`/`skill`), including
  structured outcome evidence via `task` action `outcome`
  (classification + bounded evidence + authority, no transition).
  Degraded mode:
  server unavailable → continue with native client tools; stale →
  explicit `stale`/`STALE_INDEX`; unknown → explicit `UNKNOWN`.
- Root authorization (P8 security boundary closure,
  `docs/evolution/P8_SECURITY_BOUNDARY_CLOSURE.md`): a CodeBro operation
  may only access a filesystem root explicitly authorized for the server
  process — the server root (`--root` / `CODEBRO_WORKSPACE_ROOT` / cwd,
  always authorized) plus operator-declared extras (repeatable
  `--allow-root <path>` flags and/or the `CODEBRO_ALLOW_ROOTS` env var,
  frozen at launch, immutable for the process lifetime). A per-call
  `workspace_root?` tool argument is discovery (which authorized root
  the call addresses), never authorization: it must canonicalize to
  exactly one authorized root or the call is refused with bounded
  `invalid_params` (-32602). Exact-root (never prefix-based) semantics;
  nested/overlapping roots may both be authorized deliberately and keep
  separate state; symlinks resolve to their canonical targets; existence,
  git structure, prior observation, task/skill/memory/history text, and
  client identity never authorize a root. Omitting `workspace_root?`
  addresses the server root (unchanged).

## Versioning scheme

| Change | Compatibility | Version action |
|--------|---------------|----------------|
| New optional argument | Additive | minor |
| New tool | Additive | minor |
| New field in a response object | Additive | minor |
| Removing/renaming an argument or field | Breaking | major |
| Narrowing accepted value domains (e.g. enum removal) | Breaking | major |
| Changing a semantic guarantee (e.g. trust model) | Breaking | major |

`.codebro` state files follow the same discipline; writers may only add
fields carrying serde defaults (see `facts.json` `file_digests`, memory
schema `1.1.0`).

## Error contract

All tools return JSON-RPC errors via MCP:

| Situation | Error class |
|-----------|-------------|
| Invalid arguments (unknown kind, empty key, path traversal, ambiguous/stale edit) | `invalid_params` (-32602) |
| Unauthorized `workspace_root?` (per-call root does not canonicalize to an operator-authorized root; bounded message, no host disclosure, zero side effects) | `invalid_params` (-32602) |
| Runtime/store failures (persist errors, backend unavailable) | `internal_error` |
| Unknown tool / method | rmcp standard codes |

Deterministic zero-result responses (e.g. fact search misses) are **not**
errors — they return structured payloads with recovery hints.

## Frozen tool inventory (17 frozen + additive additions: `context`, `remember`, `forget`, `recall`, `learn`, `skill`, `task`, `engineering_brief`)

> Additive optional request arguments accepted since the freeze (backwards
> compatible): `apply_change` responses carry advisory/test-recommendation
> fields; `sandbox_test` accepts `test_filter`; verification payloads include
> `diagnostics`, `classification`, `affected_modules`,
> `related_recent_changes`, and — on failures other than denial — a
> deterministic `root_cause` block of ranked evidence-backed hypotheses.
>
> P6 (engineering intelligence, `docs/evolution/P6_IMPLEMENTATION.md`):
> no new tools (still 24). Additive response fields only:
> `workspace_context` gains `repository_identity`, `index_freshness`,
> `architecture.summary`, `supported_languages`; `impact_analyze` gains
> `risk` (HIGH/MEDIUM/LOW + indicators + blast radius); `reindex` gains
> `incremental` (added/deleted/modified lists capped at 100 entries each
> with exact `added_count`/`deleted_count`/`modified_count` totals,
> `unchanged_count`, and a `truncated` flag; a failed run keeps
> `index_status: FAILED` while preserving last-good counts) + `index_status`;
> `repository_health` gains `engineering_health` / `engineering_languages`
> checks (evidence-based findings, no scores).
>
> Freshness honesty: live `freshness` compares the stored generation hash
> against the current working-tree hash (tracked files + diff + untracked
> names and bounded contents, excluding derived `.codebro/` output). Outside
> git repositories the revision signal is unavailable, so freshness reports
> `unknown` (never fabricated `fresh`); reindex when in doubt.
>
> P7 (decision support, `docs/evolution/P7_IMPLEMENTATION.md`):
> one new semantic tool (`engineering_brief`, 25 total). No CRUD, no
> ranking/embedding/model system, no new storage (schema stays v7).
>
> P8 (OpenCode integration layer, `docs/evolution/P8_IMPLEMENTATION.md`):
> NO new tools (still 25) and no schema change (v7). Product changes are
> integration-correctness fixes + client observability: stdout is
> reserved for JSON-RPC (tracing/indexer report on stderr), `initialize`
> reports `serverInfo: codebro/<version>`, and every tool call emits one
> bounded stderr observability line (client identity, tool, duration,
> status, bytes — redacted, never persisted). The 25-tool surface is the
> codified client contract (`crates/mcp-server/src/integration.rs`).

### Additive tools (minor version)

| Tool | Added | Arguments | Result highlights |
|------|-------|-----------|-------------------|
| `context` | v1.3-dev (P0 of the persistent-context evolution, `docs/evolution/`) | `workspace_root?`, `task?`, `keywords[]?`, `task_id?` (P1: task-scoped overrides resolve when the task is named) | always-available context packet: repository orientation + per-kind fact counts, task-relevant facts/decisions/memory/evidence, and resolved durable context records (per-namespace task > project > global winners + actionable intents, capped at 8), each tagged `kind`/`scope`/`authority` ∈ {user_confirmed, ai_inferred, observed, project_derived, imported, system_derived} with `effective_confidence` and decoded `intent` block where applicable; no task = clearly-labelled structural digest; read-only — never writes project or user state; response bounded at 256 KiB |
| `remember` | v1.3-dev (P1, `docs/evolution/P1_IMPLEMENTATION.md`) | `content`, `namespace`, `kind?` (default preference), `scope?` (default project), `task_id?` (required for task scope), `authority?` + `user_confirmed?` (USER_CONFIRMED requires the flag), `original_text?`, `language?`, `evidence_event_ids[]`, `observation?` (mints cited evidence event), `confidence?`, `importance?`, `source?`, `related_ids[]`, `rationale?`/`priority?`/`intent_status?` (intent only), `supersedes?`, `workspace_root?` | semantic persist of a confirmed preference or intent with provenance/scope/lifecycle; fresh namespace clashes refused naming the incumbent (replace via `supersedes`); terminal intent statuses retire the predecessor; secrets redacted; serializes on the workspace mutation lock |
| `forget` | v1.3-dev (P1, `docs/evolution/P1_IMPLEMENTATION.md`) | `id?` or `namespace?` (+`task_id?` for task rows), `confirm=true` (required), `permanent?` (default reversible reject), `workspace_root?` | reversible retirement of a context record (row stays for audit) or hard removal with `permanent=true`; project/task rows only from their own workspace; serializes on the workspace mutation lock |
| `recall` | v1.3-dev (P2, `docs/evolution/P2_IMPLEMENTATION.md`) | `query` (required, ≥1 searchable token), `scope?` (project default \| task + `task_id?` \| global opt-in), `kinds?` (taxonomy subset), `session_id?`, `limit?` (default 10, max 50), `workspace_root?` | query-driven historical evidence: session-grouped bounded excerpts (decision/failure/validation/change) with session, timestamp, scope, task, event type, source, stale flag; provenance `historical-evidence`; task history invisible without its task; read-only — recalls write nothing, history never enters `context` |
| `learn` | v1.3-dev (P3, `docs/evolution/P3_IMPLEMENTATION.md`) | `action` (run \| propose \| list \| get \| evaluate \| confirm \| reject), `scope?` (project default \| task + `task_id?` \| global opt-in with broader-evidence bar), `candidate_id?` (get/evaluate/confirm/reject), `status?`/`limit?` (list), `user_confirmed?` (confirm requires true — the model can never self-confirm), `confirm?` + `reason?` (reject), `workspace_root?` | cautious hypotheses from history: deterministic detection (token-pair clustering, ≥3 support, chatter/sensitive topics refused) → outcome-aware evaluation (support vs contradict, bounded confidence) → accepted hypotheses persist as `AI_INFERRED` records (evidence-bound, decaying, expiring — never `USER_CONFIRMED`); confirm promotes via supersede, reject preserves negative knowledge; mutating actions serialize on the workspace lock; list/get read-only; no learn action writes history |
| `skill` | v1.3-dev (P4, `docs/evolution/P4_IMPLEMENTATION.md`) | `action` (discover \| propose \| inspect \| validate \| approve \| reject \| deprecate \| rollback \| health), `candidate_id?`, `skill_id?`, `name?`, `description?`/`purpose?`/`content?` (propose; description/purpose secret-redacted at write, content additionally secret-scanned at validation), `learning_candidate_id?` (propose from accepted P3 learning), `scope?` (project default \| task + `task_id?` \| global), `languages?`/`subsystems?` (applicability), `confidence?` (standalone only; capped below the approval floor), `status?`/`limit?` (discover), `reason?` (reject/deprecate), `version?` (rollback), `success?` (health), `user_confirmed?` (approve requires true), `workspace_root?` | skill lifecycle with layered trust gates: proposals carry evidence citations (accepted learning only — weak/rejected learning refused at the store), `validate` runs the automated evaluation pass (candidate → evaluating → draft → validated), `approve` publishes only with `user_confirmed=true` AND store gates (validated status, confidence ≥ 0.60, workspace match, no name conflict, no stale version anchor) — the model can never self-approve; publication is atomic (temp+fsync+rename), symlink-safe, read-before-write (external SKILL.md edits are conflicts, never overwritten), and DB rows commit in one transaction; published versions are immutable (unique `(skill_id, version_number)`); rollback publishes old content as a new version (no-op rollbacks refused); deprecation removes the artifact so OpenCode stops discovering it; every action enforces workspace/scope visibility (task rows need `task_id`); mutating actions serialize on the workspace lock; CodeBro never executes skills — OpenCode discovers `~/.config/opencode/skills/<name>/SKILL.md` (or `$CODEBRO_SKILLS_DIR`) natively |
| `task` | v1.3-dev (P5, `docs/evolution/P5_IMPLEMENTATION.md`; P9 outcome action, `docs/evolution/P9_IMPLEMENTATION.md`) | `action` (list default \| stale \| create \| inspect \| start \| pause \| resume \| checkpoint \| validate \| validation_result \| complete \| fail \| cancel \| outcome \| skill_refs), `task_id?`, `title?`/`description?`/`priority?` (create), `intent_record_id?`/`parent_task_id?` (soft references), `idempotency_key?` (explicit dedup: same (workspace, key) returns the same task), `based_on_version?` (optimistic-concurrency anchor — stale writers refused), `status?`/`limit?` (list), `summary?`/`progress?`/`next_action?`/`metadata?` (checkpoint), `what?` (validate), `result?` (validation_result: passed \| failed), `reason?` (validation evidence / outcome / failure / cancellation text; outcome: bounded evidence detail), `changed_areas?` (complete; outcome: bounded file references), `classification?` (outcome: success \| partial \| failure \| rejected \| superseded), `user_confirmed?` (outcome: explicit user speech act — otherwise recorded as observed), `dedup_key?` (outcome: explicit idempotency — same (task, key) returns the original event), `exit_code?` (outcome: reported command exit status), `what?` (outcome: test/build command identity), `summary?` (outcome: required bounded summary), `skill_refs?` (association; reference-only; secret-redacted at write), `workspace_root?` | durable engineering task runtime: strict lifecycle (pending → running → paused/validating → completed/failed/cancelled, matrix store-enforced, callers never set status), completion gate (`complete` requires a recorded passed validation — `validation_result failed` returns the task to running), immutable versioned checkpoints (new progress = new version; task pointer moves in the same transaction — never a dangling reference), worker leases with fencing (per-process worker ids; takeover after TTL expiry fences the version forward; a stale worker can never overwrite the newer owner), interrupted tasks are stale-and-recoverable only via explicit `resume` (never auto-completed, never deleted), bounded resume snapshot on `inspect` (task + latest checkpoint + ≤10 recent events + skills + intent note), P9 `outcome` records structured outcome evidence as a task-bound `task_outcome` history event (any task status incl. terminal, no transition, no lease, no row mutation; authority `observed` unless `user_confirmed=true`; per-task-namespaced `dedup_key` idempotency; superseded stored polarity-neutral so abandonment never reads as success; feeds P3 learning as validation-group evidence and future briefs/recall as history), workspace isolation at every seam, every free-text field (incl. skill refs) secret-redacted, every transition/checkpoint/outcome writes a P2 history event (outcome in an IMMEDIATE transaction so concurrent cross-process writers serialize; P3 learning consumes task outcomes as ordinary evidence without trust bypass); request-driven — no scheduler/daemon; OpenCode remains the executor; mutating actions serialize on the workspace lock |
| `engineering_brief` | v1.3-dev (P7, `docs/evolution/P7_IMPLEMENTATION.md`) | `task?` (ad-hoc, never persisted), `task_id?` (read-only P5 snapshot; cross-workspace reads as unknown), `target_path?` / `target_symbol?` / `target_module?` (explicit; ambiguous names reported, never guessed), `keywords?`, `depth?` (default 1, max 2), `workspace_root?`; requires ≥1 scoping signal | bounded deterministic decision-support brief: repository identity + live/persisted freshness, task-relevant files/symbols/dependencies, one bounded impact traversal with risk signals, relevant tests (impact linkage + module containment; honest unknown when unlinked), task-relevant health findings, recall excerpts (never transcripts), engineering memory, accepted learning as `AI_INFERRED` (rejected only as negative knowledge), skill applicability info (never execution), read-only task state, hard vs observed constraints, decisions with currency + conflicts, evidence-based risks, explicit unknowns (`STALE_INDEX`, `NO_RELEVANT_TESTS`, …); deterministic ordering, per-section bounds, 256 KiB envelope; read-only — no mutation lock, no history writes, no lifecycle transitions |

### Read tools

| Tool | Arguments | Result highlights |
|------|-----------|-------------------|
| `workspace_context` | – | project identity, workspace root, per-kind fact counts (incl. `languages`, `frameworks`, `entry_points`), P6: `repository_identity` (canonical root + VCS), `index_freshness` (live + persisted v7 status), `architecture.summary`, `supported_languages` (parsed vs file-level + limitation) |
| `engineering_facts` | `query` (required unless `kind`/`path`), `kind` ∈ {workspace, module, package, symbol, test, build_target, dependency, relationship, reference, diagnostic, architecture_rule, language, framework, entry_point}, `path`, `limit ≤ 50` | deterministic ranked records: score desc → kind → name → path; provenance summary; freshness |
| `engineering_memory` | `task_keywords[]`, `active_file_tags[]` | bounded entries (≤20, token budget 500, min confidence 0.3) ranked importance → confidence; expired/superseded entries excluded; explicit truncation markers |
| `memory_stats` | – | entry count, tag distribution, average confidence, oldest/newest |
| `impact_analyze` | `target`, `target_type` {symbol,file,module,package}, optional `depth ≤ 5`, `direction`, `relationship_types[]`, `max_nodes` (default 1000) | status (incl. ambiguity matches), direct/transitive relationships each carrying `confidence` ∈ [0,1] (verified 0.95 / heuristic 0.55 / unknown 0.35, ×0.85 per hop), `reason`, evidence records, affected tests/modules/packages, completeness, traversal metadata, freshness, P6: `risk` (level + ≤8 sorted indicators + blast-radius summary; signals, not guarantees) |
| `sandbox_status` | – | backend {local,opensandbox}, availability, capability descriptor |
| `repository_health` | – | per-check results over workspace/.codebro/identity/facts/memory/git, P6: + `engineering_health` (CYCLE/HIGH_FANOUT/HIGH_FANIN/ORPHAN/UNRESOLVED_REFERENCE/STALE_INDEX/MISSING_TEST_ASSOCIATION/LARGE_MODULE findings with severity/evidence/location/confidence) + `engineering_languages` |

### Write tools

| Tool | Arguments | Guarantees |
|------|-----------|------------|
| `record_memory` | `key` ≤256ch, `value` ≤64KB (secret-redacted), `tags[]` ≤32×64ch, `confidence`, `importance`, `source?`, `expires_in_secs?`, `session?` | upsert by key replaces full logical entry preserving id/created_at; key conflicts supersede prior entry with lineage; near-duplicates flagged; writes never touch the fact store |
| `delete_memory` | `key`, `confirm=true` (else no-op) | deleting missing keys errors |
| `update_identity` | description/constraints/decisions/roadmap/sprint/conventions/patterns/architecture_summary | list fields append unique entries; duplicates reported skipped; requires existing identity; every free-text field is secret-redacted at write (same redaction authority as remember/task/record_memory) |
| `apply_change` | `path`, `old` (empty = create), `new` | single-file guarded mutation: boundary, traversal denial, symlink escape prevention, stale-content protection, ambiguity rejection; response includes `recommended_tests` (edited-symbol linkage, cap 32) and invalidation advisory |
| `apply_changes` | `changes[]{path,old,new}` | multi-file transaction: validate all against current content → conflict pass re-checks staleness across the set → sequential apply with rollback on first failure; all-or-nothing |
| `sandbox_exec` | `command` (read-only build/test/lint policy), `working_directory?`, `timeout?`, `metadata?` | fail-closed execution; full evidence envelope below |
| `sandbox_build` | same contract as sandbox_test (auto: cargo check/go build/npm-pnpm-yarn run build) | build/check verification |
| `sandbox_test` | `command?`, `test_filter?` (targeted names; cargo/go/pytest runners), `expected_exit_code?`, `expected_success?`, `affected_fact_ids?` | auto-detected runner + pass/fail verification with violations, parsed `diagnostics`, coarse `classification`, `affected_modules`, `related_recent_changes` (in-session edit correlation), and deterministic `root_cause` hypotheses on failures |
| `consult` | `question`, `provider` {auto,conductor}, `mode` {architecture,debugging,code_review,planning,research,second_opinion}, file contexts, context flags | opinion injection of engineering context; never mutates state |

### Execution evidence envelope (v1)

Every `sandbox_*` execution returns:

```json
{
  "execution": {
    "command", "requested_command", "resolved_command",
    "working_directory",
    "exit_code", "success", "duration_ms",
    "timestamp", "execution_id",
    "stdout", "stderr",
    "timeout", "cancelled", "denied", "denied_reason",
    "backend", "mode",
    "repo_identity": {"project_id","root","repository_type"},
    "repo_state": {"commit_sha","working_tree_dirty","working_tree_hash"},
    "sandbox_capabilities": {...},
    "reproducibility": "deterministic|likely_deterministic|non_deterministic|unknown",
    "environment": {"os","arch","family"}
  },
  "verification": {"verified", "summary", "violations", ["impacted_fact_ids"]}
}
```

## Stability guarantees enforced mechanically

1. Dependency direction is validated by `scripts/check_workspace_deps.sh`.
2. The legacy quarantine guards live in `crates/mcp-server/tests/`.
3. Trust-model separation is pinned by `tests/trust_separation.rs`
   (memory writes cannot touch `facts.json`).
4. Determinism: identical input trees produce byte-identical `facts.json`
   (regression-tested), including warm parse-cache runs.
