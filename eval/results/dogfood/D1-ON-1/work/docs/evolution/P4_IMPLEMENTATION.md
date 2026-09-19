# P4 — Skills Lifecycle Implementation

## Overview

P4 extends P3's learning pipeline into a skill lifecycle: evidence-backed skill candidates, versioned publication, and lifecycle management. CodeBro owns the skill lifecycle; OpenCode executes skills natively via its `SKILL.md` system.

## Architecture

```text
P3 LEARNING CANDIDATE
       ↓
SKILL CANDIDATE (evidence-backed proposal)
       ↓
   VALIDATION
       ↓
   DRAFT (proposed SKILL.md content)
       ↓
   APPROVED (user or trusted principal confirms)
       ↓
   ACTIVE (published to OpenCode skill filesystem)
       ↓
   UPDATED / DEPRECATED
```

### Source of Truth

- **Skill content** (SKILL.md body): lives in `~/.config/opencode/skills/<name>/SKILL.md`
- **Skill lifecycle** (status, versions, evidence, health): lives in `state.db`
- **Skill discovery**: OpenCode scans the filesystem; CodeBro provides applicability metadata via MCP

## Schema (v5 migration)

Three new tables added to `~/.codebro/state.db`:

### `skill_candidates`
Evidence-backed proposals for new or updated skills. Fields: candidate_id, workspace_root, task_id, scope, name, description, purpose, applicability_json, source_learning_candidates_json, supporting_json, contradicting_json, proposed_content, status, confidence, validation_json, created_at, updated_at, expires_at, eval_reason, rejection_reason, supersedes_skill.

### `skills`
Active skill registry. Fields: skill_id, workspace_root, scope, name, description, applicability_json, current_version, status, confidence, health_json, source_candidate_id, superseded_by, created_at, updated_at.

### `skill_versions`
Immutable published versions. Fields: version_id, skill_id, version_number, content, content_hash, source_candidate_id, supporting_json, validation_json, author, status, created_at, parent_version.

## Types

### SkillScope
- `Global` — applies across all projects
- `Project` — applies to one workspace
- `Task` — applies to a specific task

### SkillCandidateStatus
`Candidate → Evaluating → Draft → Validated → Approved → Active`
Alternative: `Rejected`, `Deferred`, `Superseded`, `Expired`, `Deprecated`

### SkillStatus
`Draft → Validated → Approved → Active`
Alternative: `Deprecated`, `Superseded`

### SkillVersionStatus
`Draft → Validated → Approved → Active`
Alternative: `Replaced`, `RolledBack`

## MCP Tool: `skill`

23rd MCP tool. Actions:

| Action | Description |
|--------|-------------|
| `discover` | List skill candidates and *active* skills visible from the workspace (task-scoped rows need `task_id`) |
| `propose` | Create a skill candidate (from accepted learning or standalone; standalone confidence is capped below the approval floor) |
| `inspect` | View candidate or skill details with versions (workspace/scope-checked) |
| `validate` | Automated evaluation pass: re-validate content and advance candidate → evaluating → draft → validated |
| `approve` | Publish skill to the OpenCode skills dir — **requires `user_confirmed=true`** |
| `reject` | Reject a candidate (preserved for audit with reason) |
| `deprecate` | Retire an active skill and remove its published file |
| `rollback` | Revert to a previous version by publishing it as a new version |
| `health` | Record usage outcome (success/failure; active skills only) |

## Safety Guards

- **No self-approval**: `skill approve` requires `user_confirmed=true` (caller-principal speech act, MCP layer) AND store-layer gates (status must be `validated`, confidence ≥ 0.60, workspace match). Standalone (non-learning) proposals carry no evidence citations: their confidence is capped below the approval floor, so only evidence-backed learning can ever publish.
- **Learning trust boundary**: only `accepted` P3 learning candidates (evaluated, evidence-weighed) can seed skill candidates; rejected/deferred/weak learning is refused at the store layer.
- **Workspace isolation**: every mutating action (approve/reject/validate/deprecate/rollback/health) verifies the requesting workspace against the skill/candidate's workspace at the store layer; every read (discover/inspect) filters by scope (global/project/task; task rows need the task id).
- **No blind file mutation**: every publication reads the current SKILL.md first and verifies its content hash against the recorded active version; external edits, missing files, and unowned files are surfaced as conflicts, never overwritten.
- **Stale-writer protection (optimistic concurrency)**: candidates record the skill version they were validated against (`based_on_version`); approval refuses when the lineage has advanced (a second author published in between) with a `stale candidate` error.
- **Atomic publication**: SKILL.md is written to a temp file, fsynced, then renamed into place; a crash can leave at most a `.tmp-<name>` residue, never a partial SKILL.md. The file publishes before the DB rows commit in one transaction.
- **Path traversal protection**: skill names are validated against OpenCode's `^[a-z0-9]+(-[a-z0-9]+)*$` rule *before* any path is constructed — traversal-shaped names are refused at propose time and structurally cannot form escape paths. Publication refuses symlinked directories and symlinked SKILL.md targets (canonicalization + symlink_metadata checks).
- **Secret scanning**: API keys, tokens, credentials blocked at propose, draft, validation, and (again) at approve.
- **Immutability**: published version rows are plain INSERT + unique `(skill_id, version_number)` index — tampering with a published version row is a hard DB error. Version updates flow only through new versions; health counters live on the skill row, never on version rows.
- **Rollback = new version**: rollback publishes the old content as a new version number; the rolled-over version rows are preserved (`replaced`/`rolled_back`), the active pointer moves forward, and no-op rollbacks (identical content) are refused.
- **Deprecation removes the artifact**: a deprecated skill's SKILL.md is deleted so OpenCode stops discovering it; version history stays for audit. Deprecated lineages refuse re-approval, rollback, and health accumulation.
- **OpenCode compatibility**: validation enforces frontmatter `name` (matching the directory) + `description` (1-1024 chars) — skills OpenCode would silently ignore are refused at validation.

The skill publication root is `$CODEBRO_SKILLS_DIR` when set (tests/embedded deployments stay hermetic), otherwise `~/.config/opencode/skills`.

## Validation

Skill content is validated for:
- YAML frontmatter presence
- Content after frontmatter
- Size limits (≤40,000 chars)
- Secret patterns (API keys, tokens, credentials)
- Name validity (alphanumeric + hyphens/underscores)
- Standard sections (Purpose, When to Use, Procedure)

## Health

Skills accumulate health metadata:
- `success_count` / `failure_count`
- `last_used_at` / `last_validated_at` / `last_failed_at`
- Degraded when failure ratio ≥ 40% with ≥ 3 uses

## Traceability Chain

```text
skill → skill_candidate → learning_candidate → evidence → event → session
```

## Test Coverage

19 context-runtime unit tests covering:
- Status transitions
- Health assessment
- Content validation
- ID determinism
- Candidate CRUD
- Lifecycle transitions
- Skill CRUD
- Version lifecycle
- Health recording
- Scope isolation
- Deprecation
- Learning candidate derivation

## Limitations

- P4 does NOT implement skill execution (OpenCode owns that)
- P4 does NOT implement a durable task runtime (that's P5)
- P4 does NOT automatically activate skills (approval required)
- P4 does NOT create skills from every repeated action (evidence thresholds apply)
- Cross-process publication is serialized by the version-slot unique index and
  the stale-version anchor, but two `codebro serve` processes against the same
  state.db can still race on the *filesystem* step (last writer wins the file,
  DB stays consistent); the documented deployment is one server per state.db.
- DB-then-file ordering: the file publishes first, then DB rows commit in one
  transaction. A crash between the two leaves a published file with no DB
  backing — surfaced by the next publish of the same lineage (which refuses
  with "no skill lineage owns it" / drift errors) rather than silently
  overwritten. Recovery is manual (inspect + remove the stray file).
- Static validation is structural + pattern-based; it does not prove a skill is
  semantically safe. The approval gate is the trust boundary, not the lint.
