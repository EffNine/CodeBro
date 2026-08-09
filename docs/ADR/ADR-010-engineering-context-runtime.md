# ADR-010: EngineeringContext Runtime

**ADR Number:** ADR-010
**Title:** EngineeringContext Runtime — Canonical Runtime Contract
**Author:** CodeBro Engineering
**Status:** Accepted
**Created:** 2026-08-09
**Updated:** 2026-08-09
**Supersedes:** None
**Related RFC:** None

---

## 1. Context

### 1.1 Background

Sprint 21 introduced Prompt Builder v2, which successfully compiles engineering
prompts. However, the public API requires a large number of individual parameters:

```rust
compile(
    system_prompt,
    project_name,
    project_info,
    intent_plan,
    relevant_files,
    conversation,
    memories,
    arch_rules,
    fact_count,
    diagnostics,
    active_files,
    user_request,
    context_budget_remaining,
)
```

This parameter explosion creates several problems:

1. **Tight coupling** — every new subsystem must understand and pass all 13+ parameters.
2. **Fragile APIs** — adding a new field requires updating every call site.
3. **No shared contract** — each subsystem constructs its own view of the context.
4. **Future subsystems** (Project Identity, Engineering Memory, Task Graph,
   Reflection, Learning Engine) would each need their own parameter lists.

### 1.2 Constraints

- Must not break existing `compile(13+ params)` API (backward compatible).
- Must be deterministic — no `HashMap` ordering, no runtime randomness.
- Must support `serde` serialization for future persistence.
- Must be immutable once built.
- Must not implement Sprint 23 features (Persistent Identity, Memory Engine,
  Reflection, Task Graph, Learning Engine, Planning Engine).

### 1.3 Stakeholders

- **Prompt Builder** — primary consumer of the new contract.
- **Context Assembly** — produces the fragment list that seeds the context.
- **Project Identity** — will read from the contract in Sprint 23.
- **Engineering Memory** — will read from the contract in Sprint 23.
- **Provider Runtime** — will read runtime metadata from the contract.
- **Future Intelligence Systems** — will implement `EngineeringContextProvider`.

---

## 2. Decision

### 2.1 Decision Statement

Introduce `EngineeringContext` as the canonical runtime contract: a single,
immutable, serializable type that carries all state required to execute one
engineering task. `PromptBuilder` gains a `compile_context(&EngineeringContext)`
method. Subsystems read from the context; they never mutate it.

### 2.2 Rationale

A canonical contract eliminates parameter explosion by giving every subsystem
a single type to depend on. It also:

- Provides a clear extension point (`EngineeringContextProvider` trait).
- Enables future persistence (serialization is built in).
- Enforces immutability at the type level.
- Makes diagnostics and statistics first-class citizens.

### 2.3 Principles Applied

- **Engineering First** — context is the primary engineering artifact.
- **Deterministic by Default** — sorted vectors, no hash maps, stable serialization.
- **Project Awareness** — `ProjectIdentity` and `WorkspaceContext` are first-class fields.
- **Provider Agnostic** — `RuntimeContext` carries metadata without coupling to any provider.
- **No Feature Creep** — only the runtime contract is introduced; no new subsystems.

---

## 3. Consequences

### 3.1 Positive Consequences

- Subsystems depend on one type instead of 13+ parameters.
- Future subsystems can implement `EngineeringContextProvider` without coupling.
- Diagnostics and statistics are always available.
- Serialization enables future persistence and testing.
- Builder pattern enforces validation at construction time.

### 3.2 Negative Consequences

- New module (`engineering_context/`) adds to the crate surface.
- ~~Existing `compile(13+ params)` still exists; both APIs coexist.~~ The
  legacy `compile(13+ params)` API was removed in ADR-012; `compile_context`
  is the sole public compile entry point.
- Builders add indirection compared to direct field assignment.

### 3.3 Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| API surface | ~~Two `compile` methods exist~~ | `compile_context` is the sole public compile API (legacy `compile` removed in ADR-012) |
| Module count | New `engineering_context/` module | Small, focused module; well-documented |
| Immutability | Builders must create new contexts | Clone-on-write is unnecessary; builders are cheap |
| Serialization | Adds dependency on `serde` | Already a dependency; no new crates |

### 3.4 Impact on Architecture

| Module | Impact |
|--------|--------|
| `prompt_builder` | ~~Gains `compile_context()` method; existing `compile()` preserved~~ `compile_context()` is the only compile entry point (ADR-012 removed `compile()`) |
| `assembly` | Produces fragments that seed `EngineeringContext` |
| `providers` | Reads `RuntimeContext` for provider metadata |
| Future modules | Implement `EngineeringContextProvider` trait |

### 3.5 Impact on Future Work

- **Sprint 23** — Project Identity and Engineering Memory will read from
  `EngineeringContext` instead of constructing their own views.
- **Sprint 24+** — Reflection, Task Graph, and Learning Engine will implement
  `EngineeringContextProvider`.

---

## 4. Alternatives Considered

| Alternative | Description | Pros | Cons | Why Rejected |
|-------------|-------------|------|------|--------------|
| A: Expand existing `Context` | Add fields to `src/context/builder.rs` | Minimal new code | Already has `#![allow(dead_code)]`; tightly coupled to old pipeline | Doesn't match the canonical contract vision |
| B: Tuple struct | Pass `(project, workspace, memory, ...)` tuple | Simple | No named fields; hard to extend | Violates clarity principle |
| C: Trait-based context | Define `ContextTrait` with accessor methods | Flexible | Indirection; harder to serialize | Over-engineered for current needs |
| D: EngineeringContext (chosen) | Single immutable struct with builder | Clear, serializable, validated | New module | Best balance of clarity and extensibility |

---

## 5. Implementation Notes

### 5.1 Code Patterns

```rust
// Canonical construction
let context = EngineeringContextBuilder::new()
    .project(project_identity)
    .task(intent_plan)
    .workspace(workspace_context)
    .memory(memory_context)
    .constraints(constraint_context)
    .runtime(runtime_context)
    .user_request(request)
    .system_prompt(prompt)
    .build()?;

// Canonical compilation
let prompt = compiler.compile_context(&context);
```

### 5.2 Anti-Patterns

```rust
// DON'T: pass individual fields
compiler.compile(system, name, info, intent, files, conv, mem, rules, count, diags, active, req, budget);

// DO: pass the context
compiler.compile_context(&context);
```

### 5.3 Migration Steps

1. All code uses `EngineeringContextBuilder` and `compile_context`.
2. ~~Existing code continues to use `compile(13+ params)`.~~ Removed in ADR-012;
   `compile_context(&EngineeringContext)` is the only public compile entry point.

---

## 6. References

- [Engineering Context Architecture](../../architecture/engineering_context.md)
- [Sprint 21 — Prompt Builder v2](../history/sprint-21.md)
- [Constitution — Engineering First](../principles/engineering_first.md)

---

## 7. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-09 | Created | CodeBro Engineering |
