# Engineering Objective

**Document:** `docs/architecture/engineering_objective.md`
**Sprint:** 27.0 — Engineering Objective & Lazy Execution
**Status:** Active

---

## 1. Purpose

CodeBro reasons about *why* it is doing a task, *where* the task fits in the
project, and *what the smallest correct change is*.

```text
END GOAL
    ↓
PROJECT VISION
    ↓
CURRENT OBJECTIVE
    ↓
CURRENT MILESTONE / SPRINT
    ↓
CURRENT TASK
    ↓
CURRENT ACTION
```

The project may know everything. The model sees only what it needs. This
document describes the compact objective model, its authority, persistence,
task integration, context integration, token strategy, and goal alignment.

---

## 2. Goal Hierarchy

The hierarchy is represented by the compact `EngineeringObjective` struct in
`src/engineering_objective/objective.rs`:

| Level | Field | Example (CodeBro) |
|-------|-------|-------------------|
| End goal | `end_goal` | Build a terminal-native engineering intelligence runtime. |
| Project vision | `project_vision` | CodeBro is the most trustworthy, transparent, and configurable engineering intelligence runtime for professional developers. |
| Current objective | `current_objective` | Make CodeBro capable of maintaining software projects. |
| Current milestone | `current_milestone` | Sprint 27 — Engineering Objective & Lazy Execution. |
| Success criteria | `success_criteria` | Compact criteria for the current objective. |
| Non-goals | `non_goals` | Boundaries that must never be crossed. |

The model never receives full documents. It only sees the compact block
produced by `EngineeringObjective::render_compact` (≈100–300 tokens).

> **The workspace objective is optional.** The example values above are
> illustrative. CodeBro never invents an objective for a workspace and never
> installs its own product goal into an arbitrary repository. When
> `.codebro/engineering_objective.json` is absent, the objective stays empty
> and unconfigured, and task execution proceeds normally.

---

## 3. Authority Model

Goal information has an explicit **declared authority hierarchy**:

```text
Product Vision
    > Architecture / ADR
    > Current Objective
    > Sprint / Milestone
    > Task
    > Temporary Memory
```

- Values come from the repository's project documentation
  (`docs/vision/`, `docs/architecture/`, `docs/ADR/`, roadmap, and the
  current sprint definition). They are never invented.
- Each objective records an optional `source` pointer (provenance metadata)
  to the authoritative document.
- This is a **declared hierarchy, not a dynamically enforced
  source-resolution engine**. The runtime does not claim that a stale
  objective automatically overrides authoritative repository documents.
  Future source reconciliation is explicitly deferred.

### Document authority tags

The model knows the origin of each piece of context:

| Tag | Origin |
|-----|--------|
| Project Vision | `docs/vision/` |
| Architecture Decision | `docs/ADR/` |
| Constraint | project identity / `docs/vision/NON_GOALS.md` |
| Engineering Memory | `.codebro/engineering_memory.json` |
| Task State | current task / intent plan |
| Temporary Tool Result | live tool output (task-scoped) |

---

## 4. Persistence

The objective persists per workspace at
`<workspace_root>/.codebro/engineering_objective.json`
(`src/engineering_objective/storage.rs`), mirroring the project identity
pattern:

| File | Content |
|------|---------|
| `engineering_objective.json` | Versioned `EngineeringObjective` snapshot |

`EngineeringObjectiveRuntime` (`src/engineering_objective/provider.rs`):

1. **Load** — reads the persisted file, verifying workspace root and schema.
   When no file exists, the objective remains empty and unconfigured.
2. **Create / install_default** — explicit, opt-in writes only. The runtime
   never calls these automatically; a missing objective is never guessed or
   persisted.
3. **Snapshot** — returns an immutable `EngineeringObjective` per task.

Persistence is explicit only (`persist()`), matching engineering memory
conventions. The runtime never writes during task execution, and the model
never silently rewrites the objective.

### Objective freshness

A persisted objective can become stale (e.g. a `current_milestone` that names
an earlier sprint). CodeBro does **not** silently rewrite or auto-synchronize
it with sprint documentation during task execution. Persisted state is
preserved until explicitly updated. Future objective/document synchronization
is deferred.

---

## 5. Task Integration

`CanonicalRuntime` (the production orchestrator) places the objective in the
task flow:

```text
Task
  ↓
Project Identity          → one immutable snapshot per task
  ↓
Engineering Objective     → snapshot + goal alignment
  ↓
Engineering Memory        → resolve_for_task (ranking + token budget)
  ↓
Context Assembly          → intent, fragments, ranking, budget
  ↓
EngineeringContext        → objective + goal_alignment + conversation
  ↓
Prompt Builder            → compact objective section
  ↓
Provider Router           → IntelligentProviderRouter
  ↓
Provider Runtime          → breaker → health → retry
  ↓
Result
```

The task's conversation is task-scoped, bounded, relevant, and budgeted
(`bounded_conversation`): at most `max_conversation_messages` recent
messages within `max_conversation_tokens`. The purpose is *"what happened
during this engineering task?"*, not *"everything the user ever said."*

---

## 6. Context Integration

`EngineeringContext` exposes the objective as a compact field rather than
flattening dozens of fields:

```rust
EngineeringContext {
    project,          // ProjectIdentity snapshot
    objective,        // EngineeringObjective (compact)
    goal_alignment,   // Option<GoalAlignment>
    task,             // IntentPlan
    workspace,
    memory,
    constraints,
    runtime,
    ...
}
```

### Always-on context

The following is always included in the prompt (high-value, low-token):

```text
Project
End Goal
Current Objective
Current Milestone
Current Task
Critical Constraints
Task Alignment
```

Only fields that are actually configured are included. If the objective is
unconfigured, the objective section is omitted entirely — no placeholder
bloat.

### On-demand context

Stored objective data is **not** automatically injected into every prompt.
The following are contextual/on-demand and may be retrieved when relevant:

```text
PROJECT VISION
SUCCESS CRITERIA
NON-GOALS
AUTHORITATIVE DOCUMENTS
RELEVANT ADRs
```

This is a documentation boundary, not a new retrieval engine: the always-on
block stays compact, and richer objective detail is treated as on-demand
without speculative infrastructure.

This is rendered by the Prompt Builder's `engineering_objective` section and
targets roughly 100–300 tokens.

---

## 7. Token Strategy

The existing token budget system remains authoritative
(`assembly::budget`, `RuntimeContext.budget_tokens`). The objective receives
a small reserved budget (`LazyExecutionPolicy.objective_budget_tokens = 300`).

Conceptually:

```text
Total Budget
│
├── Objective / Identity   ← reserved, always-on, compact
├── Task
├── Architecture
├── Memory
├── Relevant Code
├── Tool Results
└── Response History
```

If the budget becomes constrained, the priority is:

1. Preserve task
2. Preserve objective
3. Preserve critical constraints
4. Preserve directly relevant code
5. Preserve high-value architecture
6. Drop low-value memory
7. Drop unrelated context

Never sacrifice the current task to preserve historical information.

---

## 8. Goal Alignment

`GoalAlignment` is lightweight, deterministic metadata answering *"does this
task support the current objective?"*. It is **not** an ML score.

| Value | Meaning |
|-------|---------|
| `Direct` | Task directly advances the current objective |
| `Supporting` | Task supports the current objective |
| `Weakly Related` | Task is only weakly related |
| `Unclear` | Alignment could not be determined (`⚠ Task alignment unclear`) |

Alignment is computed by `EngineeringObjective::align_task` (deterministic
token overlap) and never blocks execution. It informs awareness; it never
overrides user intent.

---

## 9. Lazy by Default

The objective model is paired with the `LazyExecutionPolicy`
(`src/engineering_objective/alignment.rs`), encoding the execution
philosophy:

> CodeBro prefers the smallest correct change, reuses existing project
> capabilities, avoids speculative abstractions, validates its work, and
> stops when the requested outcome is achieved.

The runtime follows `Inspect → Understand → Retrieve → Reuse → Change →
Validate → Stop`.

`ChangeScope` classifies work as `Required` / `Recommended` / `Unrelated`.
This is **advisory guidance only**. Token overlap is a lexical heuristic,
never semantic authorization: a weak lexical match must never authorize
destructive or high-impact behavior. Consequential actions always require
explicit confirmation.

## 10. Interaction — Recommend, don't interrogate

CodeBro behaves like an experienced engineer operating inside a real
repository:

- Infer obvious engineering intent from the task and repository context.
- Execute low-risk, clearly implied engineering actions without unnecessary
  confirmation.
- Suggest a preferred approach when multiple valid options exist; explain
  meaningful trade-offs only when they matter.
- Request confirmation for destructive, irreversible, externally
  consequential, or high-impact actions.
- Ask only when genuinely blocked by missing information.
- Stop when the requested outcome is achieved and validated.

The default system prompt encodes this contract. It does not instruct the
model to "always explain" or "ask for clarification" on routine steps.

---

## 11. References

- [Sprint 27 definition](../sprints/../README.md) (mission)
- [ADR-013 — Engineering Objective & Lazy Execution](../ADR/ADR-013-engineering-objective-and-lazy-execution.md)
- [CodeBro Vision](../vision/CODEBRO_VISION.md)
- [Project Identity](../vision/PROJECT_IDENTITY.md)
- [Non-Goals](../vision/NON_GOALS.md)
- [Final Audit — Sprint 27](./engineering_objective_final_audit.md)
