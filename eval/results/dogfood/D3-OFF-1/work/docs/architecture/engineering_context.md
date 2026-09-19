# Engineering Context Runtime

**Sprint:** 22.0
**Status:** Accepted
**Created:** 2026-08-09

---

## Responsibilities

`EngineeringContext` is the single source of truth describing everything required
to execute one engineering task. It is consumed by every subsystem in CodeBro:

- **Prompt Builder** — compiles the context into a deterministic prompt
- **Project Identity** — reads project metadata
- **Engineering Memory** — reads resolved memory fragments
- **Context Assembly** — produces the fragment list that seeds the context
- **Task Graph** — reads intent and constraints
- **Provider Runtime** — reads provider, budget, and runtime metadata
- **Reflection** — reads diagnostics and statistics
- **Future Intelligence Systems** — extension point via `EngineeringContextProvider`

---

## Architecture

### Before (Sprint 21)

```
Context Assembly
    ↓
Prompt Builder
    ↓
compile(13+ parameters)
```

### After (Sprint 22)

```
Context Assembly
    ↓
EngineeringContext (immutable runtime state)
    ↓
Prompt Builder
    ↓
compile_context(context) → CompiledPrompt
```

---

## Core Types

| Type | Purpose |
|------|---------|
| `EngineeringContext` | Immutable container for all runtime state |
| `EngineeringContextBuilder` | Fluent builder with validation |
| `ProjectIdentity` | Project name, language, framework, build system |
| `WorkspaceContext` | Filesystem root, relevant files, git/package flags |
| `EngineeringMemoryContext` | Resolved memory entries with confidence tiers |
| `ConstraintContext` | Architecture and engineering constraints |
| `RuntimeContext` | Provider, budget, temperature, seed, stream flags |
| `EngineeringContextDiagnostics` | Build-time observability snapshot |
| `EngineeringContextStatistics` | Aggregate metrics after construction |

---

## Ownership

- `EngineeringContext` **owns** immutable runtime state.
- Subsystems **read** from it; they **never mutate** it.
- Mutations happen **only** through `EngineeringContextBuilder` or future runtime services.
- Once built, `EngineeringContext` is thread-safe and cloneable.

---

## Lifecycle

```
1. EngineeringContextBuilder::new()
2. .project(project_identity)
3. .task(intent_plan)
4. .workspace(workspace_context)
5. .memory(memory_context)
6. .constraints(constraint_context)
7. .runtime(runtime_context)
8. .context_fragment(fragment)   // repeated
9. .active_file(path)           // repeated
10. .user_request(request)
11. .system_prompt(prompt)
12. .build() → Result<EngineeringContext, ContextBuildError>
```

---

## Builder Pattern

```rust
let context = EngineeringContextBuilder::new()
    .project(ProjectIdentity::new("my-project", "rust"))
    .task(IntentPlan {
        detected_goal: "fix auth bug".to_string(),
        intent_type: "Execution".to_string(),
        confidence: 0.95,
        ambiguity: false,
        ambiguity_reason: None,
    })
    .workspace(WorkspaceContext::new(".").with_git(true))
    .memory(EngineeringMemoryContext::new().with_entries(vec![]))
    .constraints(ConstraintContext::new())
    .runtime(RuntimeContext::new().with_provider("openai", "gpt-4"))
    .user_request("Fix the auth bug")
    .system_prompt("You are CodeBro")
    .build()?;
```

---

## Runtime Flow

```
User Request
    ↓
Context Assembly (existing pipeline)
    ↓
EngineeringContext (built from assembly result + project identity + memory + constraints)
    ↓
PromptBuilder.compile_context(&context)
    ↓
CompiledPrompt → Provider Runtime
```

---

## Serialization

All types implement `serde::Serialize` and `serde::Deserialize`.
This enables:

- Future persistence (session restore, context caching)
- Diagnostic logging
- Testing with serialized fixtures

---

## Diagnostics

`EngineeringContextDiagnostics` captures at build time:

| Field | Description |
|-------|-------------|
| `creation_time` | ISO 8601 timestamp |
| `build_duration_ms` | Wall-clock build time |
| `fragment_count` | Number of context fragments |
| `memory_count` | Number of memory entries |
| `constraint_count` | Number of constraints |
| `workspace_files` | Number of workspace files |
| `estimated_tokens` | Token estimate |
| `provider` | Provider name (if set) |
| `template` | Intent template (if set) |

---

## Statistics

`EngineeringContextStatistics` exposes after construction:

| Field | Description |
|-------|-------------|
| `file_count` | Workspace file count |
| `memory_entries` | Memory entry count |
| `constraint_entries` | Constraint count |
| `workspace_size` | Total bytes of workspace files |
| `context_fragments` | Fragment count |
| `estimated_tokens` | Token estimate |
| `compile_time` | Build duration in ms |

---

## Validation Rules

The builder validates:

1. **Project identity is required** — `MissingProjectIdentity`
2. **Task (intent) is required** — `EmptyTask`
3. **Workspace root path is non-empty** — `InvalidWorkspace`
4. **No duplicate fragments** (same source + content length) — `DuplicateFragment(source)`

Validation can be skipped with `.with_skip_validation()`.

---

## Trait: EngineeringContextProvider

```rust
pub trait EngineeringContextProvider {
    fn provider_name(&self) -> &str;
    fn read_context(&self, ctx: &EngineeringContext) -> Result<(), String>;
}
```

Future modules implement this trait to integrate with the runtime contract
without direct coupling.

---

## Future Extension Points (Sprint 23+)

The following are **documented only**; they are **not implemented** in this sprint.

| Extension | Description |
|-----------|-------------|
| Persistent Project Identity | Load/snapshot project identity from disk |
| Engineering Memory Engine | Full CRUD for memory entries with persistence |
| Reflection Engine | Analyse context quality and suggest improvements |
| Task Graph | Represent multi-step tasks as a DAG |
| Learning Engine | Record outcomes and improve future context assembly |
| Planning Engine | Decompose complex tasks into sub-tasks |

---

## Constitution Compliance

| Principle | Compliance |
|-----------|------------|
| Engineering First | Context is the engineering artifact; everything flows from it |
| Deterministic by Default | No HashMap ordering; sorted vectors; stable serialization |
| Project Awareness | `ProjectIdentity` and `WorkspaceContext` capture project state |
| Provider Agnostic | `RuntimeContext` carries provider metadata but doesn't depend on any |
| Explainability | `Diagnostics` and `Statistics` provide full observability |
| Progressive Disclosure | Builder exposes fields incrementally; defaults are sensible |
| No Feature Creep | Only the runtime contract is introduced; no new subsystems |
