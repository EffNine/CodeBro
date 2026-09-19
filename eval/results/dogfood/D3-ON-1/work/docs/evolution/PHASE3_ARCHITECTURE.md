# Phase 3 — Architecture (brother-in-coding evolution)

Date: 2026-09-06. Compatibility: additive to the frozen v1.2 baseline; no rewriting of existing architecture.

## Challenges to the brief (resolutions)

1. **"Unify all state in ~/.codebro/state.db"** — rejected for existing stores. facts.json / engineering_memory.json / project_identity.json are frozen, verified, and contract-backed. Resolution: SQLite hosts all *new* user-context domains; existing JSON stores remain canonical and are **joined at retrieval time**. No domain lives in two stores.
2. **"Session history / messages"** — CodeBro behind MCP never sees OpenCode chat. Resolution: CodeBro records its own observed engineering activity (tool calls, mutations, verification outcomes, RCA, consults) as an event stream with session clustering. Transcript bridging is a future OpenCode-plugin concern outside this repo (schema reserves `source=message` seam).
3. **"Autonomous learning"** — constrained to CodeBro-observable evidence. Semantic inference of user language ("jangan overengineer benda ni" → "prefer simplest reasonable implementation") happens in the OpenCode model; CodeBro persists *structure, provenance, evidence* (canonical semantic statement + original text + language tag). CodeBro stays deterministic memory, not a second reasoner.
4. **Skills** — execution stays OpenCode's. CodeBro owns authoring, validation, versioning, evaluation. Publishing writes spec-compliant SKILL.md into OpenCode's global skill dir. External edits are never clobbered (drift guard).
5. **Hermes feature filter (do NOT copy)**: per-platform profiles, compaction, approval reimplementation, kanban executor, daemon pools, chat/notifications.

## Domain model (new crate: `context-runtime`)

```
ContextRecord — the durable knowledge unit for all new domains (one table, typed)
  id, record_type (PREFERENCE|INTENT|PATTERN|EXPERIENCE|PRINCIPLE|STYLE|TASTE|...),
  kind_namespace (e.g. "fp.communication.verbosity", "intent.migration.portability"),
  content (canonical SEMANTIC statement), original_text, language ("en"|"ms"|"manglish"|...),
  authority (USER_CONFIRMED|AI_INFERRED|OBSERVED|PROJECT_DERIVED|IMPORTED|SYSTEM_DERIVED),
  confidence (decays without evidence), importance, scope (GLOBAL|PROJECT|TASK),
  project_id (xxHash of root, RepoIdentity scheme), status (ACTIVE|SUPERSEDED|EXPIRED|REJECTED),
  lifecycle stage (OBSERVED→INFERRED→CONFIRMED), evidence [event_ids], supersedes, related_ids,
  source, import_origin, created_at/updated_at/expires_at
Event       — append-only observation log (kind, tool, workspace, outcome, capped 8 KiB payload)
Session     — durable clustering of events per workspace, idle-close gap, context_snapshot at open
Skill*, Task — P4/P5 tables (reserved)
FTS5        — content index over records; BM25 blended with the existing deterministic lexical scorer
```

Domain homes: FACT/DECISION/CONSTRAINT keep living in facts / identity / engineering_memory; the record table references them via `related_ids`. No duplication.

## Storage

- `~/.codebro/state.db` (user-level; per-workspace data via project_id/workspace_root columns; `CODEBRO_STATE_DIR` override for tests/deployments).
- WAL + busy_timeout (multi-process safe), single connection per process behind a mutex (mirrors mutation_lock discipline).
- Migrations: sequential `PRAGMA user_version` steps; corruption → quarantine rename (core pattern) + fresh reopen.
- Retrieval behind a trait (`ContextRetriever`); default = structured filters + FTS5 BM25 + lexical blend + authority/decay weighting. Vector reranker swappable later without touching the domain model.

## Context record lifecycle & provenance gates

- Write gates: AI_INFERRED / OBSERVED require ≥1 evidence event id; USER_CONFIRMED requires explicit caller flag; PROJECT_DERIVED only from indexer/identity pipelines.
- OBSERVED → INFERRED → CONFIRMED: confirmation writes a new record that supersedes the weaker one (auditable trail, like memory confidence adjustments).
- Confidence decays at retrieval time without fresh evidence; never permanently high.
- Portability: records export as JSON with full envelope; IMPORTED records always distinguishable; no provider/LLM lock-in.

## Fingerprint hierarchy (not a stereotype)

GLOBAL context records → PROJECT identity (existing) → TASK-scope records. Resolution per kind_namespace: task > project > global. Project decisions never auto-promote to global preferences; globals reach projects only as overridable candidates.

## Learning loop (P3)

```
events → learn() candidate formation (deterministic: ≥3 repeated evidence, change→verify→fix,
         rejected approaches, RCA hits) → typed EXPERIENCE auto-persist (SUCCESS/FAILURE/REJECTED,
         evidence-bound, reversible) → PREFERENCE/PATTERN/PRINCIPLE → PROPOSED candidates
         → promoted only via explicit confirm (supersede + evidence trail)
```
Invariants mirroring the RCA engine: never promoted into facts/identity/engineering-memory; serialized evidence == scored evidence; reversible.

## Skill lifecycle (P4)

```
pattern → SkillCandidate → validate (agentskills.io frontmatter lint, ≤500 lines, threat scan)
→ checkpoint (git shadow store, GIT_DIR trick) → commit (versioned + metadata: evidence, confidence,
  usage_count, success_rate) → publish to ~/.config/opencode/skills/<name>/SKILL.md
```
Safety: only `skill commit` mutates; external-edit drift guard refuses clobber; rollback via checkpoint.

## MCP surface (additive minor-version; existing 17 untouched)

`context` (read, P0): compose always-available packet — identity digest + facts/decisions/memory/evidence + relevant context records, provenance-tagged, bounded.
`recall` (read, P2): FTS5 historical search over sessions/events/experiences/records + joins.
`remember`/`forget` (write, P1): high-level semantic writes to user-context store with authority gates; engineering notes stay on record_memory.
`learn` (write, P3): candidate formation/evaluation + typed experience persistence.
`skill` (write, P4): list/inspect/candidate/commit/rollback/usage.
`task` (both, P5): durable state-machine records driven by OpenCode; no executor.
~7 new tools total. Permissions ride OpenCode's native model (e.g. `codebro_skill_*: ask`).

## Task runtime (P5) & automation (P6)

Tasks: id/parent/owner/project/state (PENDING→RUNNING→PAUSED→RESUMED→VALIDATING→COMPLETED|FAILED|CANCELLED)/context_snapshot/transcript refs/artifacts/recovery_state — durable records only. Engineering automation (scheduled reindex/CI analysis/dependency health/regression detection/reports) later via OS cron + `codebro` CLI verbs; no generic personal-assistant features (Telegram/Discord/voice etc.).

## Safety / migration / compatibility

- Existing 17 tools, JSON schemas, `.codebro` files: byte-identical behavior. Additions only.
- New crate `context-runtime` depends on `core` only (+ rusqlite) → dependency direction preserved.
- Composition lives in mcp-server (thin adapters, consistent with engineering_context.rs).
- `bounded_response` applied uniformly on new paths (and existing gaps closed as hardening).
- Trust-separation integration tests extended: context-runtime cannot write facts/memory/identity.
