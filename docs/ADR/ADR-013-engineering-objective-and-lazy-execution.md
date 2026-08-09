# ADR-013: Engineering Objective & Lazy Execution

**ADR Number:** ADR-013
**Title:** Engineering Objective & Lazy Execution
**Author:** CodeBro Engineering
**Status:** Accepted
**Created:** 2026-08-09
**Updated:** 2026-08-09
**Supersedes:** None
**Related:** ADR-010 (EngineeringContext), ADR-011 (Project Identity), ADR-012 (Architecture Consolidation)

---

## 1. Context

CodeBro executes tasks in arbitrary repositories. Today the runtime knows
*what* it is doing (the task) but has no first-class notion of *why* it is
doing it, *where* the task fits in the project, or *what the smallest correct
change is*. Two consequences follow:

1. **Context bloat** — the runtime must either send everything (full docs,
   full roadmap, all ADRs, all memory) or nothing. Both are wrong.
2. **Scope drift** — without a project-direction signal, the agent invents
   abstractions, refactors unrelated code, and keeps going after the outcome
   is achieved.

This ADR makes project goals first-class and defines the lazy execution
discipline.

### Constraints

- Must extend the existing canonical architecture
  (ProjectIdentityRuntime, EngineeringMemoryRuntime, ContextAssembler,
  EngineeringContext, PromptBuilder, TaskGraph, CanonicalRuntime).
- Must NOT create a new memory/context/task/planning system.
- Must stay deterministic and serializable.
- Must keep the always-on goal context small (100–300 tokens).

---

## 2. Decisions

### 2.1 Goals are first-class

Introduce `EngineeringObjective` — a compact, structured hierarchy
(`end_goal`, `project_vision`, `current_objective`, `current_milestone`,
`success_criteria`, `non_goals`) persisted per workspace. The model sees the
distilled block, never the source documents.

**Why:** Awareness of the project's direction is a prerequisite for relevant
context selection and smallest-correct-change execution. Without it, every
task is locally correct and globally arbitrary.

### 2.2 Project knowledge persists beyond sessions

The objective lives in `.codebro/engineering_objective.json`, loaded once per
task alongside project identity. It survives restarts.

**Why:** A stateless agent re-derives the same conclusions every session.
Persistent, authoritative goals make behavior stable across sessions and
teams.

**The workspace objective is optional.** When no objective file exists,
CodeBro does **not** invent goals and does **not** install its own product
objective into an arbitrary repository. The objective stays empty and
unconfigured, and task execution proceeds normally.

### 2.3 Full project context must NOT be sent on every request

Only the compact objective block, relevant fragments, and budgeted
task context reach the model.

**Why:** Full-document injection exceeds context windows, raises cost, and
drowns the model in irrelevant tokens, degrading output quality.

### 2.4 Relevant-context selection is required

The objective hierarchy influences retrieval (current task → current
objective → relevant architecture → relevant ADRs → relevant memory →
relevant files). The existing ContextAssembler and EngineeringMemory resolver
remain the retrieval path.

**Why:** Context quality is a function of relevance, not volume.

### 2.5 Lazy engineering reduces technical debt

`LazyExecutionPolicy` encodes *smallest correct change*, scope control
(`Required`/`Recommended`/`Unrelated`), reuse preference, and a stop
condition. These are advisory rules carried by the execution contract (the
canonical prompt), not a new execution engine.

**Why:** Each speculative abstraction and unrelated refactor compounds into
maintenance debt. The lazy discipline keeps the codebase minimal and
reviewable.

**Safety:** lexical heuristics are advisory, never semantic authorization. A
weak token match must never authorize destructive or high-impact behavior;
consequential actions still require explicit confirmation.

### 2.6 "Smallest correct change" beats "smallest patch"

A tiny patch that violates the architecture is technical debt; a slightly
larger change that preserves architecture, correctness, and maintainability
is preferable — but it must be explained.

**Why:** The goal is a healthy codebase, not a line count.

### 2.7 CodeBro stops after the requested outcome is achieved

Once validation passes and the outcome reflects the task, execution stops.
No unsolicited follow-up refactoring.

**Why:** Unrequested work erodes trust and increases review burden. The
developer decides what else matters.

### 2.8 Recommend, don't interrogate

CodeBro infers obvious engineering intent and executes low-risk, clearly
implied actions without unnecessary confirmation. It requests confirmation
only for destructive, irreversible, externally consequential, or high-impact
actions, and asks only when genuinely blocked by missing information.

**Why:** An engineering runtime that asks permission for every routine step
interrogates the user instead of helping. Confirmation is reserved for what
actually matters.

---

## 3. Consequences

### 3.1 Positive

- The agent knows the project's direction without its full history.
- Context is compact and relevant; budget stays authoritative.
- Scope creep is a first-class, testable policy, not a hope.
- `ai_runtime::RuntimeRouter` and `IntelligentProviderRouter` no longer look
  like competing authorities (documented boundary; only the latter is
  production).

### 3.2 Negative

- A new module (`engineering_objective/`) joins the crate surface.
- Default objective values must be kept in sync with the repository docs.
- Goal alignment is heuristic; it can mislabel — by design it never blocks.

### 3.3 What must NOT be recreated

- No second project-knowledge store alongside `project_identity` /
  `engineering_objective`.
- No second context model alongside `engineering_context`.
- No second prompt entry point alongside `compile_context`.
- No second routing authority alongside `provider_runtime`.

---

## 4. Alternatives Considered

| Alternative | Why Rejected |
|-------------|--------------|
| Store full documents in the objective | Bloat; defeats compact always-on context |
| Derive goals ad hoc from the task each call | Stateless; contradicts persistent identity |
| Build a "ProjectBrain" knowledge engine | Explicitly forbidden; duplicates existing systems |
| Flatten goals into many context fields | Violates the compact objective contract |

---

## 5. References

- [Engineering Objective architecture](../architecture/engineering_objective.md)
- [Final Audit — Sprint 27](../architecture/engineering_objective_final_audit.md)

---

## 6. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-09 | Created | CodeBro Engineering |
