# ADR-012: Architecture Consolidation — Canonical Ownership

**ADR Number:** ADR-012
**Title:** Architecture Consolidation — Canonical Ownership
**Author:** CodeBro Engineering
**Status:** Accepted
**Created:** 2026-08-09
**Updated:** 2026-08-09
**Supersedes:** None
**Related:** ADR-010 (EngineeringContext), ADR-011 (Project Identity), ADR-008 (Intelligence Platform), ADR-004 (Reliability Layer)

---

## 1. Problem

CodeBro accumulated overlapping abstractions across successive sprints. For
several responsibilities there was no single obvious owner:

- **Context**: `src/context/` (v0.3 token-budget builder) coexisted with
  `engineering_context` (ADR-010) and `assembly` (Sprint 20).
- **Prompt compilation**: `src/prompt/` (legacy string assembly) coexisted with
  `prompt_builder` (Sprint 21). `prompt_builder` itself exposed both the legacy
  `compile(13 params)` and the canonical `compile_context(&EngineeringContext)`.
- **Memory / project knowledge**: `intelligence/memory/` (`IntelligenceMemory`)
  duplicated the project-knowledge role of `project_identity` (ADR-011) and the
  fact model of `engineering_facts`.
- **Provider reliability**: `reliability/` contained health monitoring and a
  circuit breaker that duplicated `provider_runtime` (P13-P17), which is the
  documented owner of provider health, circuit breaking, retry, routing, and
  failover.
- **Task / workflow**: `agent/task_graph` (agent execution DAG) and
  `workflow_engine` (P6.4 approval-gated planning) both model
  id + dependency + topological order, with no documented boundary.

Additionally, several files were compiled into the crate without ever being
declared as modules (dead, uncompiled `src/tests/*.rs` siblings) or without
any consumer (legacy `src/prompt/`).

The crate-level `#![allow(dead_code)]` in `main.rs` hid all of this from the
compiler.

---

## 2. Decisions

### 2.1 Context

```
Canonical owner: engineering_context (runtime contract) + assembly (context assembly)
Legacy implementation: src/context/ (v0.3 ContextBuilder)
Action: Removed src/context/ and its test references.
Reason: ADR-010 establishes engineering_context as the canonical runtime
        contract. src/context/ had zero production consumers (only its own
        dead sibling src/prompt/ and one integration test referenced it).
```

### 2.2 Intelligence / Memory

```
Canonical owner:
  - project_identity — persistent project engineering knowledge (ADR-011)
  - engineering_memory — curated, task-relevant memory
  - engineering_facts / fact_store — immutable code-understanding fact model
  - memory_runtime — generic in-memory tiered memory runtime
Legacy implementation: intelligence/memory/ (IntelligenceMemory)
Action: Removed intelligence/memory/ and its tests.
Reason: IntelligenceMemory persisted a project-knowledge store
        (~/.codebro/project_memory.json) that duplicated project_identity and
        engineering_facts. It had zero production consumers.
```

### 2.3 Prompt Compiler

```
Canonical owner: prompt_builder::compile_context(&EngineeringContext)
Legacy implementation: src/prompt/ and PromptCompiler/PromptBuilder::compile(13 params)
Action:
  - Removed src/prompt/ (zero consumers).
  - Removed the 13-parameter compile() / compile_with_default_template() public
    methods. compile_context(&EngineeringContext) is now the only public compile
    entry point. All tests migrated to compile_context.
Reason: ADR-010 declares compile_context canonical. The 13-parameter API had
        zero production callers; keeping it would leave a dead legacy path.
```

### 2.4 Provider Reliability

```
Canonical owner: provider_runtime (health, circuit breaker, retry, routing, failover)
Legacy implementation: reliability/health.rs + reliability/circuit_breaker.rs
Action:
  - Removed reliability/health.rs and reliability/circuit_breaker.rs (duplicates).
  - Kept reliability/{error,timeout,resource_guard,diagnostics,logging} as
    provider-agnostic generic infrastructure.
  - Migrated runtime/context.rs off reliability::HealthMonitor.
  - Deleted tests that only exercised the removed reliability health/circuit
    breaker. Coverage for provider health and circuit breaking remains in
    provider_runtime's own suites.
Reason: provider_runtime is the documented owner of provider health and
        circuit breaking (mod.rs, P17.0). Reliability's stripped-down
        re-implementations violated "one health system, one circuit breaker".
```

### 2.5 Task / Workflow

```
Canonical owner (not a duplicate):
  - agent/task_graph — live agent-execution DAG (nodes, edges, runtime status)
  - workflow_engine — stateless, approval-gated workflow planning (WorkflowPlan)
Action: None (documented boundary only).
Reason: TaskGraph owns mutable execution state and is driven by
        agent/coordinator; WorkflowEngine produces immutable plans and never
        executes. They are different layers (execution vs planning), not
        competing implementations. No code change was required.
```

### 2.6 Dead Code

```
Action: Removed uncompiled orphaned files:
  - src/tests/concurrency.rs, src/tests/p3_validation.rs, src/tests/validation.rs
  - src/memory_runtime/tests.rs
  - src/indexer/ (RepositoryIndex) — its only consumers were the removed
    legacy src/context/ module.
Reason: None of the orphaned test files were declared as modules (no mod.rs,
        no #[path]), so they were never compiled. Keeping them suggested test
        coverage that did not exist. src/indexer/ had zero remaining consumers.
```

---

## 3. Consequences

### 3.1 What was removed

- `src/context/` (legacy context builder)
- `src/prompt/` (legacy prompt assembly)
- `src/indexer/` (legacy `RepositoryIndex`; dead once `src/context/` was removed)
- `src/intelligence/memory/` (IntelligenceMemory)
- `src/reliability/health.rs`, `src/reliability/circuit_breaker.rs`
- `PromptCompiler::compile(13 params)` and `PromptBuilder::compile()/compile_with_default_template()`
- Orphaned uncompiled test files
- ~90 tests that exercised only removed abstractions (health/circuit-breaker/
  context/intelligence-memory). Coverage for retained concerns moved to or
  remained with the canonical owners.

### 3.2 What is canonical

- Engineering context: `engineering_context` + `assembly`
- Prompt compilation: `prompt_builder::compile_context(&EngineeringContext)`
- Persistent project knowledge: `project_identity`
- Task memory: `engineering_memory`; fact model: `engineering_facts`/`fact_store`;
  tiered runtime: `memory_runtime`
- Provider health / circuit breaking / retry / routing / failover: `provider_runtime`
- Generic reliability infra: `reliability/{error,timeout,resource_guard,diagnostics,logging}`
- Agent execution DAG: `agent/task_graph`; workflow planning: `workflow_engine`

### 3.3 What future contributors should use

- New context consumers depend on `EngineeringContext` (via `EngineeringContextBuilder`)
  and never construct their own ad-hoc context views.
- New prompt consumers call `compile_context(&EngineeringContext)` — never a
  parameter list.
- New project-knowledge persistence goes through `ProjectIdentityRuntime`;
  new task-relevant memory goes through `EngineeringMemoryRuntime`.
- New provider health/circuit-breaker/retry/routing logic lives in
  `provider_runtime`, never in `reliability/`.
- New generic infra (timeouts, resource guards, logging) lives in `reliability/`.

### 3.4 What must NOT be recreated

- No second context model alongside `engineering_context`.
- No second prompt compilation entry point alongside `compile_context`.
- No second provider health system / circuit breaker / routing authority
  outside `provider_runtime`.
- No project-knowledge memory store alongside `project_identity` and
  `engineering_memory`.
- No re-adding `src/tests/*.rs` orphan files — test modules must be declared.

---

## 4. Remaining Technical Debt (documented, not resolved)

| Item | Status | Rationale |
|------|--------|-----------|
| `agent/memory.rs` + `agent/memory_manager.rs` | Kept | Only consumed by the `#[deprecated]` `Agent` (`agent/agent.rs`) and tests. Removal would rewrite the agent runtime surface; deferred to a dedicated sprint. |
| TUI command dispatch vs `docs/vision/COMMAND_SYSTEM.md` | Documented | The TUI slash-command set diverges from the constitution (single-slash vs `//`/`!` namespaces, missing commands). A TUI rewrite is out of scope for a cleanup sprint. |
| `assembly`, `prompt_builder`, `engineering_context`, `project_identity`, `engineering_memory`, `provider_runtime`, `workflow_engine` (Sprint 20-24 stack) | Kept | Canonical by ADR but not yet wired into the TUI runtime path. Wiring them is a runtime integration sprint, not cleanup. |
| Provider health in `provider_manager` and `ai_runtime::router` | Kept | Production health/routing exists there; consolidating touches provider behavior (out of scope). |

---

## 5. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-09 | Created | CodeBro Engineering |
