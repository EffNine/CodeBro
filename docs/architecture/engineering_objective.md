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

---

## 3. Authority Model

Goal information must have explicit authority. The documented precedence:

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
- Each objective records an optional `source` pointer to the authoritative
  document.
- If sources conflict, the authoritative source wins per the precedence
  above. Contradictory information is never silently merged; uncertainty is
  exposed via `GoalAlignment::Unclear` (`⚠ Task alignment unclear`).

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
2. **Create / install_default** — writes the objective; the default is
   derived from the repository's documented project goals.
3. **Snapshot** — returns an immutable `EngineeringObjective` per task.

Persistence is explicit only (`persist()`), matching engineering memory
conventions. The runtime never writes during task execution.

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
(`src/engineering_objective/alignment.rs`), encoding:

> CodeBro prefers the smallest correct change, reuses existing project
> capabilities, avoids speculative abstractions, validates its work, and
> stops when the requested outcome is achieved.

`ChangeScope` classifies work as `Required` / `Recommended` / `Unrelated`.
Only `Required` is executed automatically; `Recommended` may be mentioned but
not modified without justification; `Unrelated` is left alone.

---

## 10. References

- [Sprint 27 definition](../sprints/../README.md) (mission)
- [ADR-013 — Engineering Objective & Lazy Execution](../ADR/ADR-013-engineering-objective-and-lazy-execution.md)
- [CodeBro Vision](../vision/CODEBRO_VISION.md)
- [Project Identity](../vision/PROJECT_IDENTITY.md)
- [Non-Goals](../vision/NON_GOALS.md)
- [Final Audit — Sprint 27](./engineering_objective_final_audit.md)
