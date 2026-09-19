# ADR-011: Project Identity Runtime

**ADR Number:** ADR-011
**Title:** Project Identity Runtime — Persistent Engineering Knowledge
**Author:** CodeBro Engineering
**Status:** Accepted
**Created:** 2026-08-09
**Updated:** 2026-08-09
**Supersedes:** None
**Related RFC:** None

---

## 1. Context

### 1.1 Background

Sprint 22 introduced `EngineeringContext` as the canonical runtime contract.
`EngineeringContext` intentionally does not own long-term project knowledge —
it is a per-session snapshot.

Without persistent project memory, CodeBro is a stateless chatbot that
forgets everything between sessions. Each new session starts from scratch,
re-reading files, re-discovering patterns, and re-learning conventions.

Project Identity is the engineering memory of the repository. It captures
architecture, decisions, constraints, and evolution. It makes CodeBro
progressively smarter the longer it works inside the same repository.

### 1.2 Constraints

- Must not implement Learning Engine or Reflection (Sprint 24+).
- Must not couple `ProjectIdentity` to `PromptBuilder`.
- Must be deterministic — sorted vectors, no HashMap ordering, stable serialization.
- Must persist to human-readable JSON in `.codebro/`.
- Must support schema versioning and migrations.
- Must validate on load (missing metadata, duplicates, unknown schema).
- `EngineeringContext` only consumes snapshots; it does not own persistence.

### 1.3 Stakeholders

- **EngineeringContextBuilder** — primary consumer of project identity snapshots.
- **PromptBuilder** — reads identity indirectly through EngineeringContext.
- **Future runtimes** (Engineering Memory, Reflection, Learning) — depend on `ProjectIdentityProvider` trait.
- **Users** — inspect `.codebro/` files directly for transparency.

---

## 2. Decision

### 2.1 Decision Statement

Introduce `ProjectIdentityRuntime` as the standalone runtime responsible for
loading, maintaining, validating, updating, and persisting project identity.
`EngineeringContextBuilder` consumes immutable snapshots via
`ProjectIdentityRuntime.snapshot()`. `ProjectIdentity` does not depend on
`PromptBuilder`.

### 2.2 Rationale

Project memory is more valuable than chat memory because:

1. **Cross-session continuity** — The runtime remembers the codebase, not
   just the conversation. Authentication was implemented with JWT last week;
   the next session knows this without re-discovering it.
2. **Engineering-first focus** — Project identity captures architecture,
   decisions, and constraints. Chat history captures words. The former is
   engineering knowledge; the latter is conversation log.
3. **Inspectability** — All identity data lives in human-readable JSON
   files under `.codebro/`. Users can open, edit, and understand exactly
   what CodeBro knows.
4. **Determinism** — Sorted vectors and timestamp-ignoring equality ensure
   the same repository always produces the same identity snapshot.
5. **Separation of concerns** — `EngineeringContext` owns per-session state.
   `ProjectIdentityRuntime` owns persistent state. This boundary prevents
   session data from leaking into long-term memory.

### 2.3 Principles Applied

- **Engineering First** — Identity captures engineering knowledge, not conversation.
- **Project Awareness** — Remembers architecture, decisions, constraints across sessions.
- **Deterministic by Default** — Sorted vectors, stable serialization, timestamp-ignoring equality.
- **Explainability** — Diagnostics expose every load, save, migration, and validation event.
- **Provider Agnostic** — Pure Rust persistence; no LLM coupling.
- **Core Remains Small** — Focused module; no intelligence or reflection logic.
- **No Feature Creep** — Only Project Identity Runtime. No Learning Engine, no Reflection.

---

## 3. Consequences

### 3.1 Positive Consequences

- CodeBro remembers the codebase across sessions.
- `EngineeringContext` stays lean — it consumes snapshots, it doesn't manage persistence.
- Future runtimes depend on `ProjectIdentityProvider` trait, not concrete types.
- Human-readable JSON storage enables direct inspection and editing.
- Migration pipeline supports schema evolution without breaking existing data.
- Validation catches issues early (missing metadata, duplicates, unknown versions).
- Diagnostics provide full observability into load/save/migration/update operations.

### 3.2 Negative Consequences

- New module (`project_identity/`) adds to the crate surface.
- File I/O on every load is synchronous (acceptable for identity data scale).
- Builders add indirection compared to direct field assignment.
- Migration code must be maintained as schema evolves.

### 3.3 Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| Persistence location | `.codebro/` vs global config | Per-repository isolation; matches user expectation |
| Immutable snapshots | Cloning on every `snapshot()` call | Cheap clone (String + Vec); negligible cost |
| Schema versioning | Migration code grows over time | Backward-compatible; unknown versions fail fast |
| Trait abstraction | `ProjectIdentityProvider` adds indirection | Future runtimes depend on trait, not concrete type |
| Separate storage files | 8 JSON files vs 1 monolithic file | Modularity; subsystems can read subsets |

### 3.4 Impact on Architecture

| Module | Impact |
|--------|--------|
| `engineering_context` | Consumes `ProjectIdentity` snapshot via builder; no persistence |
| `prompt_builder` | Reads identity indirectly through `EngineeringContext`; no direct coupling |
| `provider_runtime` | No change; remains decoupled from identity |
| Future runtimes | Implement `ProjectIdentityProvider` trait for integration |

### 3.5 Impact on Future Work

- **Sprint 24** — Engineering Memory Runtime will extend project identity with long-term memory.
- **Sprint 24+** — Reflection Runtime, Learning Runtime, Task Graph Runtime will depend on `ProjectIdentityProvider`.

---

## 4. Alternatives Considered

| Alternative | Description | Pros | Cons | Why Rejected |
|-------------|-------------|------|------|--------------|
| A: Embed in EngineeringContext | Add identity fields directly to `EngineeringContext` | Simpler | Context becomes a dump truck; couples session state with persistent state | Violates separation of concerns |
| B: Global singleton | Single shared `ProjectIdentity` instance | Easy access | Not thread-safe; hard to test; couples all subsystems | Violates modular architecture |
| C: SQLite backend | Store identity in a SQLite database | Queryable; ACID | Heavy dependency; overkill for JSON-scale data; not human-readable | Violates inspectability principle |
| D: ProjectIdentityRuntime (chosen) | Standalone runtime with file-based persistence | Clean separation; trait-based; human-readable; migratable | More modules; more files | Best balance of all principles |

---

## 5. Implementation Notes

### 5.1 Code Patterns

```rust
// Canonical creation
let mut runtime = ProjectIdentityRuntime::new(workspace_root);
let identity = runtime.create_minimal("my-project", "rust")?;

// Canonical loading
let identity = runtime.load()?;

// Canonical snapshot consumption
let context = EngineeringContextBuilder::new()
    .project(runtime.snapshot())
    .build()?;

// Canonical update
runtime.update(IdentityChanges {
    add_constraints: vec!["No raw SQL".to_string()],
    ..Default::default()
})?;
```

### 5.2 Anti-Patterns

```rust
// DON'T: couple identity to prompt builder
pub struct ProjectIdentity {
    pub prompt_template: String,  // ← identity should not know prompts
}

// DON'T: mutate identity from EngineeringContext
context.project.known_constraints.push(...);  // ← EngineeringContext is read-only

// DON'T: update on every prompt
runtime.update(IdentityChanges::default());  // ← empty changes, wasteful
```

### 5.3 Storage Layout

```
<workspace_root>/.codebro/
  project_identity.json          # Canonical: full identity snapshot (sole authoritative input)
  workspace.json                 # Derived projection: workspace metadata
  architecture.json              # Derived projection: architecture summary and patterns
  engineering_decisions.json     # Derived projection: recorded decisions
  constraints.json               # Derived projection: known constraints
  roadmap.json                   # Derived projection: roadmap items
  current_sprint.json            # Derived projection: active sprint
  metadata.json                  # Derived projection: schema version and timestamps
```

**Canonical-read / derived-write behavior:**

- `project_identity.json` is the **sole authoritative input**. All loads read exclusively from this file.
- The other seven files are **derived, inspectable projections** written only by the runtime. They are never read back as a source of truth.
- After every `create`, `create_minimal`, successful `update`, and successful migration, the runtime persists the complete `ProjectIdentity` to all eight files atomically (in sequence) via `storage.save_all()`.
- Manually editing a supplementary projection file has no effect on the runtime — the next `load()` reads only `project_identity.json`.
- This design prevents stale or conflicting data in the projection files from silently overriding the canonical snapshot.

---

## 6. References

- [Project Identity Vision](../vision/PROJECT_IDENTITY.md)
- [Engineering Principles](../philosophy/engineering_philosophy.md)
- [ADR-010: EngineeringContext Runtime](./ADR-010-engineering-context-runtime.md)
- [Sprint 23.0 Specification](../../sprints/sprint-23.0.md)

---

## 7. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-09 | Created | CodeBro Engineering |
