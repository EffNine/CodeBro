# Runtime Integration Audit — Final (Sprint 26)

Status: **Post-implementation audit.** Compares the actual pre-sprint runtime
path against the production path after Sprint 26 canonical runtime integration.

---

## Before

The pre-sprint production path was `run_chat_pipeline` inside `src/tui/ui.rs`:

```
TUI input
  ↓
run_chat_pipeline (tui/ui.rs)
  ├─ tool registry (direct)
  ├─ OpenAiProvider::new(config)          ← provider hard-wired
  ├─ run_tool_pipeline → tool_context      (string)
  ├─ AgentCoordinator::run_task → report    (string)
  ├─ prompt = format!("User task: ...")     ← string concatenation
  ├─ call_ai_streaming(provider, prompt)    ← direct provider call
  └─ ReAct loop with string-accumulated prompt
```

Canonical subsystems (ProjectIdentity, EngineeringMemory, ContextAssembler,
EngineeringContext, PromptBuilder, ProviderRuntime, IntelligentProviderRouter)
were complete and tested but had **zero production callers**.

## After

```
TUI input
  ↓
CanonicalRuntime::run_task()               (src/canonical_runtime/)
  ├─ TaskGraph lifecycle                    (agent::task_graph)
  ├─ ProjectIdentityRuntime.snapshot()      one per task
  ├─ EngineeringMemoryRuntime.resolve()     ranking + token budget
  ├─ run_tool_pipeline                      (observe)
  ├─ AgentCoordinator.run_task()            (reason)
  ├─ ContextAssembler::assemble()           (intent + fragments + budget)
  ├─ EngineeringContextBuilder.build()      immutable handoff
  ├─ PromptBuilder::compile_context()       canonical compiler
  ├─ IntelligentProviderRouter::route()     authoritative selection
  ├─ ProviderRuntime                        breaker → health → retry
  ├─ I/O provider stream_response()         via ProviderAdapter
  └─ ReAct loop (recompiles per iteration)
  ↓
TaskResult → TUI rendering
```

## Canonical Components Used

| Component | Module | How wired |
|---|---|---|
| `ProjectIdentityRuntime` / `snapshot()` | `project_identity/` | `CanonicalRuntime::new_from_parts` loads/creates; one snapshot per task |
| `EngineeringMemoryRuntime::resolve_for_task` | `engineering_memory/` | memory resolution before context build |
| `ContextAssembler::assemble` | `assembly/` | observe stage; sources: project info, workspace, tool results, indexer (when index exists) |
| `EngineeringContextBuilder::build` | `engineering_context/` | the handoff contract for every model-facing path |
| `PromptBuilder::compile_context` | `prompt_builder/` | sole prompt compiler in the production path |
| `IntelligentProviderRouter::route` | `provider_runtime/routing` | authoritative provider selection |
| `ProviderRuntime` (`from_parts`, `report_success`, `report_failure`, breakers, retry policy) | `provider_runtime/` | execution gates + accounting |
| `CircuitBreaker` / `CircuitBreakerRegistry` | `provider_runtime/` | gating in `stream_once`; never bypassed |
| `RetryController` / `RetryPolicy` | `provider_runtime/` | retry loop over the runtime policy |
| `TaskGraph` / `TaskStatus` | `agent/task_graph.rs` | task lifecycle (Pending/Running/Completed/Failed) |
| `AgentCoordinator` | `agent/coordinator.rs` | reason phase (report fragment) |
| `ToolRegistry` | `dispatcher/` | ReAct tool execution |
| `CodeIndexer` | `intelligence/index/` | attached to Context Assembly when an index db exists |

## Remaining Bypasses

1. **`CodeIndexerTrait::get_indexed_files` is a stub** returning `Vec::new()`
   (`intelligence/index/mod.rs:203`), so `IndexerContextSource` still yields
   zero indexer fragments. `EngineeringFactsSource` (symbols) works. The
   indexer is wired but its file-selection path is inert until the stub is
   implemented. Pre-existing gap, not introduced by this sprint.
2. **`ai_runtime` (RuntimeRouter) remains unconnected.** A separate,
   model-level router with its own `ModelCandidate` registry. The production
   path uses the provider-level `IntelligentProviderRouter`. `ai_runtime` is
   a parallel abstraction; not wired (and not required by this sprint).
3. **`integration_pipeline` + `workflow_engine` remain uncoupled from the TUI.**
   The pipeline orchestrates the P6 planning chain and workflow planning
   separately. The runtime uses `agent::AgentCoordinator` + `TaskGraph` for
   task execution. Future sprint: unify planning through the workflow engine.
4. **TUI conversation is not fed into the context.** The TUI passes an empty
   conversation; the `EngineeringContext.conversation` field is populated by
   API but unused in the production chat path (same as pre-sprint behavior).
5. **`providers::OpenAiProvider` fails open on broken URLs.** `stream_response`
   returns the receiver eagerly; connection errors surface as an empty stream
   rather than a hard failure. Pre-existing provider behavior; the runtime
   reports what it observes.

## Ownership

| Subsystem | Owner | Input | Output | Responsibility |
|---|---|---|---|---|
| TUI | `tui/` | user input, events | rendered UI | input, rendering, diagnostics visibility |
| Canonical Runtime | `canonical_runtime/` | task + config | `TaskResult` | orchestrate stages, task lifecycle, diagnostics |
| Project Identity | `project_identity/` | workspace root | `ProjectIdentity` snapshot | load/create/validate/persist identity |
| Engineering Memory | `engineering_memory/` | task keywords + tags | `EngineeringMemoryContext` | resolve ranked, budgeted memory |
| Context Assembly | `assembly/` | request + sources | `ContextAssemblyResult` | intent, fragments, ranking, budget |
| Engineering Context | `engineering_context/` | builder inputs | immutable `EngineeringContext` | the handoff contract |
| Prompt Builder | `prompt_builder/` | `&EngineeringContext` | `CompiledPrompt` | deterministic compilation |
| Provider Routing | `provider_runtime/routing` | `&RouteRequest` | `ProviderRoutingDecision` | authoritative selection |
| Provider Runtime | `provider_runtime/` | decision | gates + accounting | breaker, health, retry, cost, reporting |
| Provider I/O | `providers/` | prompt | streamed response | vendor HTTP |
| Task Graph | `agent/task_graph.rs` | task lifecycle events | graph state | lifecycle state |

## Technical Debt

- The repo still carries `#![allow(dead_code, unused_imports, unused_variables, clippy::all)]`
  in ~180 files. This sprint did not blanket-remove those allowances (most are
  LEGITIMATE for a fast-moving monolith); the goal of keeping warnings useful
  is preserved: the new `canonical_runtime/` module compiles warning-free.
- `tui/ui.rs` and `tui/app.rs` remain large; execution moved out, but further
  rendering decomposition is a future sprint.
- `ProviderAdapter` hard-codes `Streaming` + `ToolCalling` capabilities and
  zero cost. Future: derive capabilities/cost from the provider config.

## Deferred Work (next sprint)

- Implement `get_indexed_files` so the canonical `IndexerContextSource`
  produces relevant-file fragments (the indexer is already wired).
- Feed the TUI conversation into `EngineeringContext.conversation`.
- Unify the `ai_runtime` RuntimeRouter with `IntelligentProviderRouter`
  (single router abstraction) or document one as canonical.
- Route the planning phase through `workflow_engine`/`integration_pipeline`.
- Surface `TaskDiagnostics` in a dedicated verbose panel (currently emitted as
  a pipeline log).
- Provider capability/cost metadata via config-driven `ProviderAdapter`.

---

*Compare with `runtime_integration_audit.md` (the pre-implementation baseline).*
