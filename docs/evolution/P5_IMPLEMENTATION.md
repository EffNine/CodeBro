# P5 — Durable Engineering Task Runtime Implementation

## Overview

P5 introduces the durable execution-state layer: engineering work is represented
as durable task state that survives OpenCode restarts, CodeBro restarts, session
boundaries, pauses, and interruptions. A task is a bounded piece of engineering
work with a persistent lifecycle, context, checkpoints, and outcome — NOT a
chat session, a context record, a skill, or a generic TODO.

CodeBro owns the durable task state. OpenCode remains the reasoning and
execution environment: it performs the actual engineering work, records
checkpoints through CodeBro, and continues from them. P5 is request-driven
only — no scheduler, no background daemon, no polling.

## Architecture

```text
OpenCode (reasoning / coding / execution)
   │  task create / start / checkpoint / validate / complete / … via MCP
   ▼
task tool (MCP, tool 24)
   ▼
ContextStore task runtime (crates/context-runtime/src/tasks.rs)
   ├── tasks (SQLite state.db, schema v6)
   ├── task_checkpoints (immutable, unique (task_id, version))
   ├── worker lease + fencing (wkr::<hex> worker ids)
   └── every transition + checkpoint writes a P2 history event
        (same transaction; FTS-synced; P3-visible)
```

The durable loop P5 completes:

```text
Intent (P1) → Task → Checkpoints → OpenCode execution → Validation
→ Outcome → Task completion → P3 learning evidence → future context/skills
```

## Task Model

`tasks` table (schema v6):

| Field | Role |
|---|---|
| `task_id` | Opaque `task::<16 hex>`, minted store-side (time+pid+counter, existence-checked). Never a path or rowid. |
| `workspace_root` | Canonical workspace key — the isolation boundary. |
| `title` / `description` | Bounded, redacted caller text. |
| `status` | Lifecycle state (see below). |
| `priority` | Organizational only; no scheduling semantics. |
| `intent_record_id` | Soft P1 intent reference (never duplicated, never rewritten). |
| `parent_task_id` | Organizational nesting only (no DAG, no scheduling). |
| `idempotency_key` | Explicit dedup: same (workspace, key) → same task. Titles are never deduplicated. |
| `current_version` | Optimistic concurrency; bumped on every mutation. |
| `current_checkpoint_id` | Mutable pointer to the latest immutable checkpoint. |
| `lease_worker` / `lease_expires_at` / `lease_version` / `lease_heartbeat_at` | Worker ownership (see below). |
| `validation_json` | The completion-gate evidence. |
| `skill_refs_json` | Reference-only skill association. |
| `outcome_json` | Terminal outcome (completed/failed). |

## Lifecycle (strict state machine)

```text
pending → running → paused → running (resume)
                  → validating → completed (gate: passed validation)
                                → running (validation failed → fix work)
                  → failed / cancelled
pending → cancelled
```

- `resumed` is normalized to `running` (a transition, not a state).
- The transition matrix (`task_transition_allowed`) is store-enforced; callers
  never supply the next status. Impossible transitions are rejected:
  `completed → anything`, `pending → completed`, `validating → paused`, etc.
- Every transition writes exactly one history event, in the same transaction
  as the row update.

### Completion gate

`completed` requires a recorded **passed** validation result. The flow is
`running → validating` (`task validate`), then a recorded result
(`task validation_result result=passed|failed`), then `task complete`.
A failed validation returns the task to `running` (fix and retry). A caller
cannot skip validation: completion from any other state is refused, and
completion with a failed/missing result is refused at the store layer.

## Checkpoints (immutable, atomic, bounded)

A checkpoint answers: what was the task, what is completed, what remains, what
was decided, what validation ran, what failed, what happens next. It is durable
resume state, NOT a transcript.

- New progress = new version row (1, 2, …). Published checkpoints are never
  modified; the task's `current_checkpoint_id` pointer moves forward only.
- Creation is transactional: task row + checkpoint row + pointer move + history
  event + lease heartbeat commit together. The pointer can never reference a
  nonexistent checkpoint, and an event never outlives a rolled-back transition.
- Checkpoint fields are bounded (summary/progress/next_action ≤ 4096 chars,
  metadata ≤ 8192 chars, must be a JSON object) and **refused** when oversized
  (not truncated): checkpoints are caller-authored resume state — silent
  truncation would corrupt it. Free text is secret-redacted before storage.
- Terminal tasks refuse new checkpoints.

## Worker Ownership, Leases, and Fencing

- Each MCP server process mints one worker id (`wkr::<16 hex>`).
- `start`/`resume` acquire the task lease (TTL 15 min); checkpoints by
  the holder heartbeat it. Pause/terminal transitions release it in the
  same transaction.
- **Fencing**: every lease takeover (expired lease, different worker) increments
  `lease_version`. Every live-state mutation carries the caller's lease version;
  a worker whose version fell behind is refused with a `stale worker` error — a
  disappeared worker can never overwrite a newer owner's state. Exclusivity is
  not guessed from timestamps alone; the fencing token arbitrates.
- **Takeover only via explicit resume** (post-implementation audit fix):
  while a task names an owner, only that owner may mutate live state —
  even after the lease expired. A non-owner that tries to
  checkpoint/pause/validate/complete directly is refused and directed to
  `resume` first (which fences the version forward). A forged or guessed
  lease version never grants access: ownership, not the caller's number,
  arbitrates. Checkpoints on holderless (pending/paused) tasks carry no
  lease columns at all — they never invent or extend lease state.
- `task heartbeat` renews a **live** lease; only the holder can. Renewing
  an already-expired lease is refused — recovery after expiry goes
  through explicit `resume` (post-implementation audit fix).

## Interruption & Recovery

An interrupted RUNNING/VALIDATING task (dead process, expired lease) is:

- still `running` on disk — never auto-completed, never deleted,
- derived as stale at read time (`TaskRecord::is_stale`, `task stale`),
- recoverable only via **explicit** `task resume` (which fences the lease
  forward to the new worker).

Paused tasks are intentional stops, never interruptions: `is_stale` is
true only for `running`/`validating` (post-implementation audit fix —
paused tasks with a released lease were previously derived as stale).

`task inspect` (the resume snapshot) composes the bounded recovery packet:
task identity, latest checkpoint, last 10 task-scoped history events,
skill association, validation state, and the intent status note. No transcript
replay.

## Intent / Session / Context / Skill / Learning Relationships

- **Sessions (P2)**: a task survives sessions; task events carry the task id
  and sessions stay in their own table. `task_id != session_id`.
- **Intent (P1)**: soft reference. If the referenced intent is terminal
  (completed/cancelled), the task continues and `inspect` surfaces the
  mismatch as a note. CodeBro never rewrites or destroys intent history, and
  never forces the task to pause (continue-and-surface policy).
- **Context (P0/P1)**: task-scoped context records already exist; tasks do not
  duplicate them. Another task's records stay invisible (task scope requires
  the task id, store-enforced since P1).
- **Skills (P4)**: `skill_refs_json` is a reference-only association (the skill
  ids a task used). Task outcomes never mutate skill health — one task outcome
  is evidence, not proof; P4 health rules decide. CodeBro never executes
  skills; OpenCode does.
- **Learning (P3)**: task lifecycle events are ordinary P2 history events. P3
  consumes them through its existing evidence rules: `task_completed`,
  `task_validation_passed`, `task_validation_failed`, and `task_failed` map
  into the `Validation` evidence group (outcome-bearing, weight 1.0);
  structural transitions map to `Observation`. Task outcomes never bypass P3
  trust gates — no auto-promotion, no USER_CONFIRMED, evidence minimums
  unchanged.

## MCP Tool: `task` (24th tool)

| Action | Description |
|--------|-------------|
| `list` | Tasks for this workspace (status filter, bounded, checkpoint-light) |
| `stale` | Recoverable work: interrupted tasks with expired leases |
| `create` | Create a task (title, description, priority, intent/parent refs, idempotency_key, skill_refs) |
| `inspect` | Bounded resume snapshot (task + checkpoint + recent events + skills + intent note) |
| `start` | pending → running (acquires the lease) |
| `pause` | running → paused (releases the lease) |
| `resume` | paused/interrupted → running (takes over the lease, fences forward) |
| `checkpoint` | New immutable checkpoint version + pointer move |
| `validate` | running → validating (records what is being validated) |
| `validation_result` | Record passed/failed (+ evidence); failed → back to running |
| `complete` | validating → completed (gate: passed validation; writes the outcome) |
| `fail` | running/validating → failed (failure evidence preserved) |
| `cancel` | non-terminal → cancelled (supervisory; no lease needed) |
| `skill_refs` | Set the reference-only skill association |

Semantics, not CRUD: `pause` means "pause if the current owner/state permits",
never "set status=paused". The MCP layer enforces the state machine, workspace
isolation, and the mutation lock; validation errors surface as
`invalid_params`, the rest as internal errors. Responses are bounded via
`response_bounds`.

## Schema v6 Migration

Two new tables (`tasks`, `task_checkpoints`) added stepwise (v5 → v6):

- `CREATE TABLE IF NOT EXISTS` throughout — crash-safe resume.
- No backfill: tasks are created going forward; P0–P4 rows untouched.
- Unique index on `(workspace_root, idempotency_key) WHERE NOT NULL`;
  `tasks(workspace_root, status, updated_at)` listing index; unique
  `(task_id, version)` on checkpoints.
- Tested: fresh DB, v5 → v6 with skills/learning/history/records preserved,
  interrupted-migration resume, no quarantine on upgrade.

## Security

- **Workspace isolation** at MCP, storage, and retrieval layers: every read
  re-verifies the canonical workspace (cross-workspace = invisible or
  refused); every mutation re-loads the task inside the transaction and
  checks ownership. Adversarially tested at every seam.
- **Secrets**: every free-text field (title, description, checkpoint fields,
  validation what/evidence, outcome summaries, event summaries/payloads) is
  redacted via the single redaction authority (`redact_secrets_public`)
  before storage. Synthetic-secret tests cover checkpoints, DB, history,
  MCP responses, and the resume snapshot.
- **Bounds**: field-level caps with refusal (not truncation) for
  caller-authored resume state; event payloads capped by the P2 seam;
  MCP responses bounded by `response_bounds`.
- **No filesystem task state**: tasks live in SQLite only; no task JSON
  directory, no new path surface (no traversal/symlink attack surface).

## Test Coverage

- 23 store-level tests (`crates/context-runtime/src/tasks.rs`): transition
  matrix exhaustively (allowed + rejected + self), full lifecycle with event
  ordering, completion gate (both refusal paths), idempotency, opaque ids,
  workspace isolation at every seam, optimistic concurrency (pause race,
  checkpoint race), lease exclusivity, takeover + fencing (stale worker
  refused everywhere), heartbeat, interruption/recovery, bounded resume
  snapshot, checkpoint immutability/validation, redaction, skill-ref bounds,
  soft intent/parent refs, terminal-intent surfacing, P3 evidence mapping
  (kind groups + polarity + FTS), restart durability, 4-way concurrent
  checkpoint race (exactly one winner).
- 6 migration tests (schema v6): fresh shape, v5→v6 preservation, resume
  after partial application.
- 7 MCP-level tests (`task_tests`): full lifecycle over MCP, completion gate,
  invalid transitions, cross-workspace isolation, stale-writer + cross-worker
  lease enforcement, redaction over MCP, idempotent create.
- 4 real-binary E2E tests (`tests/task_runtime_e2e.rs`, actual `codebro serve`
  over stdio): lifecycle across two restarts with durable outcome, killed
  process leaves recoverable task (never auto-completed; dead worker's live
  lease fences the new process), cross-process workspace isolation, task
  events recallable through `recall`.

## Limitations / P6 Boundary

- No scheduler, cron, recurring tasks, background daemon, autonomous workers,
  distributed execution, generic queue, or notification system — P6 territory.
- The lease TTL is wall-clock (15 min); a live-but-dead worker guards its task
  until expiry, by design (fencing over liveness guessing).
- Cross-process writers rely on SQLite WAL + the documented one-server-per-
  state.db deployment; within one server, mutations serialize on the
  workspace mutation lock, and the version check arbitrates races.
- Tasks are user-context state (`~/.codebro/state.db`), scoped by workspace —
  they never enter the per-project JSON stores.
