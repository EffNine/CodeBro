# Portable Memory (WS5)

Explicit, verified, redacted portability for CodeBro's durable state.

## Scope

Portable tables (exported): `context_records`, `events`, `learning_candidates`,
`sessions`, `skill_approval_requests`, `skill_candidates`, `skill_versions`,
`skills`, `task_checkpoints`, `tasks` — the same layout the pre-existing
`~/memory/export` mirror produces.

Imported tables: `sessions`, `events`, `context_records`, `skills`,
`skill_versions`, `skill_candidates`. The remaining tables are exported for
mirror completeness and reported as `not_imported`: pending approval requests,
tasks/checkpoints, and learning candidates are device-local workflow state
that must not be silently resurrected on another machine.

Transport is explicit: `codebro export` writes a directory, `codebro import`
reads one. There is no daemon, watcher, scheduler, or cloud call.

## Format

```
<dir>/manifest.json      versioned manifest
<dir>/<table>.jsonl      one JSON object per row, keys sorted
<dir>/README.md          format note
```

`manifest.json`:

```json
{
  "format_version": 1,
  "generated_at": "1789748000",
  "data_hash": "<sha256 over the exported rows>",
  "counts": {"context_records": 8, "...": 0},
  "source_db": "/home/user/.codebro/state.db",
  "tables": {"context_records": ["id", "record_type", "..."]}
}
```

- `data_hash` is SHA-256 over sorted table names + compact JSON of each
  table's rows (keys sorted) — the same construction as the legacy mirror.
- `format_version` is absent in legacy (pre-WS5 curator) mirrors.
- Unknown manifest fields are ignored on read; a `format_version` newer than
  the binary's is refused.

## Export semantics

- **Read-only.** The SQLite connection is opened `READ_ONLY`.
- **Deterministic.** Rows are ordered by `rowid`; keys are sorted; the same
  state exports to the same bytes and the same `data_hash`.
- **Redacted.** Every string value passes through
  `redact_secrets_public` before writing. Skill-version content hashes are
  recomputed over redacted content so bundles remain self-consistent.
- Bounded file writes are atomic (`write_atomic`).

## Import semantics

Import is a two-phase controlled state transition.

**Phase 1 — load, verify, validate, plan (no writes).**

1. Parse the manifest; refuse newer formats and missing `data_hash`.
2. Parse every JSONL row (bounds: 50k rows/table, 200k total).
3. Recompute the hash over parsed rows:
   - `format_version` present → strict; mismatch refuses the import.
   - legacy manifest → hash match is `hash_verified`; otherwise row counts
     are checked and the report says `counts_verified` (foreign
     canonicalization may format floats differently).
4. Map rows into domain types and validate each one (`validate_record`,
   `validate_event`, safe skill names, known scope/status vocabularies,
   recomputed skill content hashes).
5. Remap workspaces: explicit `--map OLD=NEW` first; a single-source bundle
   defaults to `--root`. Records and skills for unmapped workspaces are
   skipped and counted. Sessions and events are evidence and are imported
   verbatim (original workspace root preserved when unmapped), so citations
   from imported global records always resolve.
6. Resolve evidence citations: each cited event id must be in the bundle or
   already local. Anything else refuses the record (provenance is never
   fabricated).
7. Decide merge actions against local rows.

Any failure in phase 1 aborts with nothing written.

**Phase 2 — apply (dependency order).**

| Table | Seam | Semantics |
|---|---|---|
| sessions | `import_session` | insert-if-absent; original timestamps; redacted/bounded free text; `event_count` starts at 0 and is recounted after events |
| events | `import_event` | redacted + bounded; content-addressed dedup key when the source had none; session linkage only when the session exists locally |
| records | `import_record` | upsert by id preserving `created_at`/`updated_at`; evidence ids remapped; `import_origin` set |
| skills / versions | `import_skill_lineage` | insert-if-absent after validation; no transition, no merge |
| candidates | `import_skill_candidate` | insert-if-absent; status preserved verbatim |
| SKILL.md | `publish_skill_file` | active/approved imported skills publish a missing artifact atomically; an existing differing file is never clobbered (warning) |

## Trust rules

- **Authority is copied verbatim.** `user_confirmed` stays confirmed;
  `ai_inferred` never becomes confirmed; nothing is promoted.
- **Never overwrite newer local state.** Imported `updated_at` older than the
  local row → skipped. Equal timestamps with different content → reported
  conflict, local row untouched.
- **Never downgrade confirmed knowledge.** A newer imported record with a
  weaker authority than the local row is a conflict, not an update.
- **Idempotent.** Records upsert by id; events dedup by content-addressed
  key; skills insert-if-absent. A second import of the same bundle is a
  no-op (duplicates counted), so an interrupted apply can be re-run safely.
- **Evidence ids are local coordinates.** Duplicate detection compares
  records modulo evidence ids (equal citation counts are equivalent
  provenance); different citation counts are a conflict.

## CLI

```
codebro export --out <dir> [--json]
codebro import --file <dir> [--root <ws>] [--map OLD=NEW]... [--dry-run]
               [--no-publish] [--origin <label>] [--json]
```

`--dry-run` performs phase 1 and reports the plan without writing data.
`CODEBRO_STATE_DIR` selects the store; `CODEBRO_SKILLS_DIR` selects the
skills root (import publishes missing artifacts unless `--no-publish`).

## Boundaries (honest)

- The bundle is **trusted input**: the hash detects corruption, not
  malicious authorship. Only import bundles from a source you trust.
- Import is a CLI/operator action. It is deliberately **not** exposed as an
  MCP tool, so no agent can trigger filesystem reads or cross-device merges.
- The external `~/memory` curator script (outside this repository) writes
  raw rows without CodeBro's export redaction; run `codebro export` when
  redaction matters.
- Concurrent writers are governed by the single-writer assumption, as
  everywhere else in the store.

## Tests

- `crates/context-runtime/src/portability.rs` — 20 unit tests: round trip,
  deterministic export, tamper refusal, idempotent re-import, evidence-remap
  duplicate detection, newer-local protection, newer-import update,
  equal-timestamp conflict, no-downgrade, redaction (records, skills, hashes),
  malformed manifest, corrupted row (nothing written), legacy
  count-verification, workspace remap/skip, dry run, artifact publication,
  future-format refusal, foreign-evidence resolution, crafted table-name
  traversal refusal.
- `crates/mcp-server/tests/portability_e2e.rs` — 4 real-CLI probes: round
  trip + idempotency, dry run, missing database, malformed manifest.
