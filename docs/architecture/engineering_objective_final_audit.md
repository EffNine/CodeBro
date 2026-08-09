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

**Workspace objective is optional.** When `.codebro/engineering_objective.json`
is absent, the objective stays empty and unconfigured. CodeBro never invents
an objective and never installs its own product goal into an arbitrary
workspace. A missing objective never breaks task execution.

Authority precedence is a **declared hierarchy** (documented ordering +
`source` provenance metadata), not a dynamically enforced resolution engine:

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
| Persistent (optional) | `.codebro/engineering_objective.json` | across sessions; explicit writes only, empty when absent |
| Persistent | `.codebro/engineering_memory.json` | across sessions |
| Task-scoped | `EngineeringContext.conversation` | per task, bounded + budgeted |
| Task-scoped | Tool results, agent analysis fragments | per task |
| Runtime-only | Provider state, breakers, latency, diagnostics | never persisted to project knowledge |

Runtime noise is never persisted into project knowledge. Persisted
objectives can become stale; CodeBro does not auto-synchronize them with
sprint documentation (deferred).

## 4. Token / Context Strategy

- Always-on objective block ≈ 100–300 tokens (design budget 300). Only
  configured fields are included; an unconfigured objective emits no section.
- Always-on vs on-demand: `project_vision`, `success_criteria`, `non_goals`,
  and authoritative documents are on-demand context, never auto-injected.
- Conversation bounded: `max_conversation_messages = 20`,
  `max_conversation_tokens = 1500`, newest-first.
- Existing budget system remains authoritative (`assembly::budget`,
  `RuntimeContext.budget_tokens`).
- Trimming order under pressure: task → objective → critical constraints →
  relevant code → high-value architecture → low-value memory → unrelated.

## 5. Lazy Execution Policy

`LazyExecutionPolicy` + `ChangeScope` (`src/engineering_objective/alignment.rs`):

- `classify_change_scope` → `Required` / `Recommended` / `Unrelated` —
  **advisory guidance only**, never semantic authorization. Lexical overlap
  must never authorize destructive or high-impact behavior.
- `prefers_reuse` → existing implementation/abstraction preferred.
- `should_stop` → stop once the outcome is achieved and validation passes.

Workflow: `Inspect → Understand → Retrieve → Reuse → Change → Validate → Stop`.
The execution contract is carried by the canonical prompt (system identity):
recommend don't interrogate, reuse first, keep scope tight, validate, then
stop.

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
| Objective provisioning | No automatic objective extraction from repository docs (future work). Missing objectives stay unconfigured. |
| Goal alignment / scope heuristics | Deliberately advisory; never block execution and never authorize destructive behavior. |
| Pre-existing clippy warnings (4) | In `ai_runtime`/`memory_runtime`; unrelated to Sprint 27, not introduced here. |
| Conversation bounding | Simple newest-first window; no explicit task-boundary markers yet. |
| Persisted objective staleness | Persisted objectives can go stale; no auto-synchronization (deferred). |

## 12. Deferred Work

- Wiring objective edits/updates into the TUI (a maintenance surface, not
  required for awareness).
- Automatic objective extraction/reconciliation from repository docs
  (explicitly out of scope for the correction pass).
- Fine-grained document retrieval ranking weighted by objective relevance
  beyond token overlap.
- A dedicated sprint to remove the self-contained `ai_runtime` module if it
  remains consumer-less.

## 13. Validation

```text
cargo fmt --check   PASS
cargo check         PASS (4 pre-existing warnings)
cargo test          PASS (2428 passed; 0 failed; 1 ignored)
cargo clippy        PASS (4 pre-existing warnings, none new)
cargo build         PASS
```

Baseline (pre-Sprint 27): `cargo test` 2361 passed. After Sprint 27:
2415 passed. After the correction pass: 2428 passed (+67 over baseline,
+13 over the initial Sprint 27 landing). No regressions.
