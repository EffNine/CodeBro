# Sprint 27 Final Architecture Audit — Engineering Objective & Lazy Execution

**Date:** 2026-08-09
**Status:** Final

---

## 1. Goal Hierarchy

```
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

Represented by `EngineeringObjective` (`src/engineering_objective/objective.rs`).
Fields: `end_goal`, `project_vision`, `current_objective`,
`current_milestone`, `success_criteria`, `non_goals`, `source`,
`schema_version`. Compact only — never stores full documents.

Authority precedence (documented and enforced by ordering + source pointer):

```text
Product Vision > Architecture / ADR > Current Objective > Sprint / Milestone > Task > Temporary Memory
```

## 2. Context Hierarchy

```
EngineeringContext {
    project          → ProjectIdentity snapshot (per task)
    objective        → EngineeringObjective (compact, always-on)
    goal_alignment   → Option<GoalAlignment> (awareness, never blocking)
    task             → IntentPlan
    workspace        → WorkspaceContext
    context_fragments→ assembled + agent-analysis + tool-result fragments
    memory           → EngineeringMemoryContext (budgeted resolution)
    constraints      → ConstraintContext (identity constraints)
    runtime          → RuntimeContext (provider, budget)
    active_files, user_request, conversation, system_prompt, diagnostics, statistics
}
```

No flattening into dozens of fields; the objective stays compact.

## 3. Persistence Boundaries

| Tier | Store | Lifetime |
|------|-------|----------|
| Persistent | `.codebro/project_identity.json` (+ projections) | across sessions |
| Persistent | `.codebro/engineering_objective.json` | across sessions |
| Persistent | `.codebro/engineering_memory.json` | across sessions |
| Task-scoped | `EngineeringContext.conversation` | per task, bounded + budgeted |
| Task-scoped | Tool results, agent analysis fragments | per task |
| Runtime-only | Provider state, breakers, latency, diagnostics | never persisted to project knowledge |

Runtime noise is never persisted into project knowledge.

## 4. Token / Context Strategy

- Always-on objective block ≈ 100–300 tokens (`LazyExecutionPolicy` budget 300).
- Conversation bounded: `max_conversation_messages = 20`,
  `max_conversation_tokens = 1500`, newest-first.
- Existing budget system remains authoritative (`assembly::budget`,
  `RuntimeContext.budget_tokens`).
- Trimming order under pressure: task → objective → critical constraints →
  relevant code → high-value architecture → low-value memory → unrelated.

## 5. Lazy Execution Policy

`LazyExecutionPolicy` + `ChangeScope` (`src/engineering_objective/alignment.rs`):

- `classify_change_scope` → `Required` / `Recommended` / `Unrelated`.
- Only `Required` runs automatically; `Recommended` requires justification;
  `Unrelated` is left alone.
- `prefers_reuse` → existing implementation/abstraction preferred.
- `should_stop` → stop once the outcome is achieved and validation passes.

Workflow: `Inspect → Understand → Retrieve → Reuse → Change → Validate → Stop`.

## 6. Canonical Runtime Integration

`CanonicalRuntime::run_task` now includes objective awareness:

```text
Task
 ↓
Project Identity            (snapshot)
 ↓
Engineering Objective       (snapshot + goal alignment)
 ↓
Engineering Memory          (resolve_for_task)
 ↓
Context Assembly            (ContextAssembler)
 ↓
EngineeringContext          (builder: objective + goal_alignment + conversation)
 ↓
Prompt Builder              (compile_context → Engineering Objective section)
 ↓
IntelligentProviderRouter   (authoritative)
 ↓
ProviderRuntime             (breaker → health → retry → stream)
 ↓
TaskResult
```

No bypass of `CanonicalRuntime`. `compile_for_task` exposes the same path for
observability/tests.

## 7. Indexer Integration

- `CodeIndexerTrait::get_indexed_files` now returns distinct indexed files
  from the symbol DB (`SymbolDatabase::list_indexed_files`, deterministic,
  sorted). Previously it returned `Vec::new()`.
- Verified: indexed files → `ContextAssembler` (Indexer source fragments) →
  `EngineeringContext` → compiled prompt.
- No new indexer (`IndexerV2` / `CodeSearchEngineV2` etc.) was created.

## 8. Tool-Result Integration

- Tool results enter the canonical pipeline as `ContextFragment` with
  `source = "tool_results"` (observe stage) or `source = "tool_result"`
  (ReAct loop via `extend_context`).
- No `Tool → String → prompt concatenation`. Verified by assembly test
  `test_assemble_flows_tool_results_into_fragments`.

## 9. Provider Router Decision

**Audit:** `ai_runtime::RuntimeRouter` vs
`provider_runtime::routing::IntelligentProviderRouter`.

| Aspect | `ai_runtime::RuntimeRouter` | `IntelligentProviderRouter` |
|--------|------------------------------|------------------------------|
| Consumers | its own tests + `AIRuntime` wrapper only | production `CanonicalRuntime` |
| Input | `ModelCandidate` (self-contained) | `RegisteredProvider` registration metadata + health/cost state |
| Role | provider-agnostic model-selection prototype | sole production provider-selection authority |
| Verdict | **KEEP (documented reference)** | **KEEP (authoritative)** |

There is no second authority in the execution path. Boundary documented in
`src/ai_runtime/mod.rs`. No removal: all consumers are identified (internal
to `ai_runtime`), and removal would touch a self-contained reference module
with no production impact — deferred.

## 10. Provider Metadata

- `ProviderAdapter` now prefers provider-declared metadata
  (`providers::Provider::capabilities()` / `cost()`) when present,
  falling back to the production defaults (`Streaming` + `ToolCalling`;
  default cost) only for legacy providers that do not self-describe.
- No new provider metadata system was created; routing consumes
  `RegisteredProvider` registration metadata as before.

## 11. Remaining Technical Debt

| Item | Notes |
|------|-------|
| `ai_runtime::RuntimeRouter` | Self-contained prototype; no production consumers. Keep + document (Sprint 27 decision); removal deferred. |
| Default objective values | Derived from repo docs; must be kept in sync as docs evolve. |
| Goal alignment is heuristic | Deliberately never blocks execution; may mislabel. |
| Pre-existing clippy warnings (4) | In `ai_runtime`/`memory_runtime`; unrelated to Sprint 27, not introduced here. |
| Conversation bounding | Simple newest-first window; no explicit task-boundary markers yet. |

## 12. Deferred Work

- Wiring objective edits/updates into the TUI (a maintenance surface, not
  required for awareness).
- Fine-grained document retrieval ranking weighted by objective relevance
  beyond token overlap.
- A dedicated sprint to remove the self-contained `ai_runtime` module if it
  remains consumer-less.

## 13. Validation

```text
cargo fmt --check   PASS
cargo check         PASS (4 pre-existing warnings)
cargo test          PASS (2415 passed; 0 failed; 1 ignored)
cargo clippy        PASS (4 pre-existing warnings, none new)
cargo build         PASS
```

Baseline (pre-sprint): `cargo test` 2393 passed. Final: 2415 passed
(+22 new tests). No regressions.
