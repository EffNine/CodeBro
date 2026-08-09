# Canonical Runtime Integration

Sprint 26 wired the canonical Sprint 20–25 subsystems into the actual
production execution path. This document describes the canonical execution
flow, component ownership, data flow, the `EngineeringContext` handoff,
provider routing, diagnostics, task lifecycle, and TUI responsibility.

The repository is the source of truth; this document is the map.

---

## 1. Canonical Execution Flow

```
User Request
      ↓
TUI (input / rendering only)
      ↓
CanonicalRuntime::run_task()        ← orchestrator; owns task lifecycle
      ↓
ProjectIdentityRuntime.snapshot()   one immutable identity per task
      ↓
EngineeringMemoryRuntime.resolve_for_task()   ranking + token budget
      ↓
tools::executor::run_tool_pipeline()          ground truth (observe)
      ↓
agent::AgentCoordinator.run_task()            analysis report (reason)
      ↓
assembly::ContextAssembler::assemble()        intent, fragments, ranking, budget
      ↓
engineering_context::EngineeringContextBuilder  → immutable EngineeringContext
      ↓
prompt_builder::PromptBuilder::compile_context()  → CompiledPrompt
      ↓
provider_runtime::routing::IntelligentProviderRouter::route()  ← authoritative selection
      ↓
provider_runtime::ProviderRuntime
      ├─ Circuit Breaker (never bypassed)
      ├─ Health reporting (report_success / report_failure)
      └─ Retry policy
      ↓
providers::Provider::stream_response()  (via ProviderAdapter)
      ↓
TaskResult → TUI rendering
```

The ReAct loop (compile → route → execute → parse tool calls → act → repeat)
lives inside the runtime. Each iteration recompiles the prompt from an updated
`EngineeringContext`, so every model call goes through the canonical path.

## 2. Component Ownership

| Concern | Owner |
|---|---|
| Task lifecycle | `canonical_runtime::CanonicalRuntime` drives `TaskGraph` (Pending → Running → Completed/Failed) |
| Project identity | `project_identity::ProjectIdentityRuntime` (load / create / snapshot) |
| Engineering memory | `engineering_memory::EngineeringMemoryRuntime` (resolve only; writes stay explicit) |
| Context assembly | `assembly::ContextAssembler` |
| Engineering context | `engineering_context::EngineeringContextBuilder` → immutable `EngineeringContext` |
| Prompt compilation | `prompt_builder::PromptBuilder::compile_context(&EngineeringContext)` |
| Provider selection | `provider_runtime::routing::IntelligentProviderRouter` (single authoritative path) |
| Provider gates | `provider_runtime::ProviderRuntime` (breaker, health, retry, cost, reporting) |
| Provider I/O | `providers::Provider` plugins (via `ProviderAdapter`) |
| Tool execution | `dispatcher::ToolRegistry` (ReAct tool calls) |
| Rendering / input | `tui/` |

## 3. Data Flow

- `ProjectIdentityRuntime` persists under `<root>/.codebro/project_identity.json`
  (plus companion files). The runtime is constructed per task; identity is
  loaded (or minimally created) once and snapshotted once per task.
- `EngineeringMemoryRuntime` persists under `<root>/.codebro/engineering_memory.json`.
  `resolve_for_task` respects the resolver's deterministic ranking
  (importance → confidence → key → id), entry budget (20) and token budget
  (500). No automatic learning, no LLM-driven writes.
- `ContextAssembler` classifies intent, collects fragments from injected
  sources (user request, project info, workspace, tool results, indexer when
  an index exists), then ranks, dedups, and applies the token budget.
- Assembled fragments + the coordinator report are mapped into
  `EngineeringContext.context_fragments`.
- `PromptBuilder::compile_context` performs deterministic template selection,
  section ordering, empty-section omission, diagnostics and statistics, and
  the context budget.
- The routed provider's id is resolved to its I/O handler for execution;
  success/failure is reported back through `ProviderRuntime`.

## 4. EngineeringContext Handoff Contract

Every engineering task reaching the model produces an immutable
`EngineeringContext` populated with:

- `project` — the ProjectIdentity snapshot
- `task` — intent plan (goal, intent type, confidence)
- `workspace` — root, relevant files, git/readme/manifest flags
- `context_fragments` — assembled + agent-analysis fragments
- `memory` — resolved engineering memory
- `constraints` — identity constraints
- `runtime` — provider/model metadata, streaming flag
- `active_files` — important files
- `user_request` / `conversation` / `system_prompt`

The model-facing path does not bypass this contract: no string-only prompt
construction, no legacy context objects.

## 5. Provider Routing

- One authoritative routing path: `IntelligentProviderRouter::route(&RouteRequest)`
  with capabilities `Streaming` + `ToolCalling`.
- Routing and the provider runtime share a `ProviderRegistry`, `HealthManager`
  and `CostTracker` (via `ProviderRuntime::from_parts`), so routing observes
  the same health/cost state that execution reports into.
- No duplicate routing logic exists in the agent/TUI layer.

## 6. Provider Runtime Gates

```
Provider Selection
      ↓
Circuit Breaker  can_execute() — rejected when open; failure reported
      ↓
Health           report_success() / report_failure()
      ↓
Retry            RetryController over the runtime retry policy
      ↓
Provider Request io_provider.stream_response(prompt)
```

The circuit breaker is never bypassed. `report_success` and `report_failure`
remain connected to health, cost and breaker accounting.

## 7. Diagnostics

Per-task `TaskDiagnostics` records:

- project, task, intent
- context fragments, memory entries, prompt tokens
- template, provider, routing strategy + reasons
- breaker state and whether the breaker allowed the request
- identity / memory / assembly / compile / routing / execution / total timings

Diagnostics are emitted as an `AgentEvent::Log` at `pipeline` level and are
surfaced in the dashboard activity log / verbose modes (progressive
disclosure). They never influence execution.

## 8. Task Lifecycle

`TaskGraph` (existing `agent::task_graph`) drives the lifecycle:

```
Pending (created) → Running (start) → Completed | Failed
```

Cancellation maps to the existing `Skipped` state (future work). The runtime
does not invent a new state machine.

## 9. TUI Responsibility

The TUI owns:

- input and command interaction
- task display and output rendering
- diagnostics visibility (activity log, agents panel, task graph panel)

The TUI does **not** own:

- context assembly, prompt compilation, provider routing/health, memory
  resolution, or project identity persistence.

`run_chat_pipeline` in `tui/ui.rs` is now a thin adapter that constructs the
canonical runtime and forwards the result to the renderer.

## 10. Performance Observations

Measured on a debug build with a mock provider and a fresh temp workspace
(`cargo test --bin codebro perf_measurement -- --ignored --nocapture`):

| Stage | Time |
|---|---|
| Runtime startup (identity create + memory + router) | ~14 ms |
| Context assembly (tools + coordinator + assembler) | ~5 ms |
| Provider execution (streaming) | ~8 ms |
| Total orchestration | ~16 ms |
| Identity snapshot, memory resolve, compile, routing | sub-ms each |

Pre-sprint the production path had no stage instrumentation, so no baseline
was recorded. No premature optimization was performed.
