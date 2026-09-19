# Project Identity Runtime — Architecture

**Version:** 1.0.0
**Status:** Active
**Date:** 2026-08-09
**Sprint:** 23.0

---

## 1. Overview

The Project Identity Runtime is the subsystem responsible for maintaining
persistent engineering knowledge about a repository across sessions.

Project Identity is NOT chat history. It is the engineering memory of the
repository — architecture, decisions, constraints, and evolution. It makes
CodeBro progressively smarter the longer it works inside the same
repository.

## 2. Architecture

```
Repository
    ↓
Project Identity Runtime
    ↓
EngineeringContext Builder
    ↓
EngineeringContext
    ↓
Prompt Builder
    ↓
Provider Runtime
```

## 3. Responsibilities

| Responsibility | Method | Description |
|---------------|--------|-------------|
| Load | `load()` | Read identity from `.codebro/` storage |
| Create | `create()` / `create_minimal()` | Build a fresh identity and persist it |
| Validate | `validate()` | Run deterministic validation rules |
| Update | `update()` | Apply engineering state changes |
| Persist | (internal) | Write to `.codebro/` JSON files |
| Snapshot | `snapshot()` | Return an immutable `ProjectIdentity` clone |

## 4. Lifecycle

### 4.1 Creation

When CodeBro encounters a new repository with no `.codebro/` directory:

1. `ProjectIdentityRuntime::create_minimal(name, language)` builds a
   minimal identity.
2. The runtime persists `project_identity.json`, `workspace.json`, and
   `metadata.json` to `.codebro/`.
3. Subsequent `snapshot()` calls return the created identity.

### 4.2 Loading

When CodeBro enters an existing repository:

1. `ProjectIdentityRuntime::load()` reads `project_identity.json`.
2. If the schema version differs from current, `apply_migrations()` runs
   the migration pipeline.
3. `validate_identity()` runs all validation rules.
4. Diagnostics record load time, migration count, and validation errors.

### 4.3 Updating

Project identity changes only when engineering state changes. Examples:

| Trigger | Change Applied |
|---------|---------------|
| Architecture analysis complete | `architecture_summary`, `known_patterns` |
| Sprint completed | `recent_milestones`, roadmap status |
| Constraint discovered | `known_constraints` |
| Decision accepted | `engineering_decisions` status update |
| Roadmap updated | `roadmap` items |
| New module discovered | `known_modules` |

Do NOT update on every prompt. Updates are explicit and purpose-driven.

### 4.4 Snapshot

`snapshot()` returns an immutable `ProjectIdentity` clone.
`EngineeringContextBuilder` calls `snapshot()` to consume project identity.
`EngineeringContext` never writes — it only reads snapshots.

## 5. Persistence Model

All files live under `<workspace_root>/.codebro/`:

| # | File | Content | Role |
|---|------|---------|------|
| 1 | `project_identity.json` | Full identity snapshot | **Canonical** — sole authoritative input |
| 2 | `workspace.json` | Workspace metadata | Derived projection |
| 3 | `architecture.json` | Architecture summary and patterns | Derived projection |
| 4 | `engineering_decisions.json` | Recorded decisions | Derived projection |
| 5 | `constraints.json` | Known constraints | Derived projection |
| 6 | `roadmap.json` | Roadmap items | Derived projection |
| 7 | `current_sprint.json` | Active sprint identifier | Derived projection |
| 8 | `metadata.json` | Schema version and timestamps | Derived projection |

### 5.1 Canonical-Read / Derived-Write

- **Canonical read**: `load()` reads exclusively from `project_identity.json`. The seven supplementary files are never consulted as a source of truth.
- **Derived write**: After every `create`, `create_minimal`, successful `update`, and successful migration, the runtime persists the complete `ProjectIdentity` to all eight files in sequence via `storage.save_all()`.
- **Inspectability**: Subsystems and users may read the supplementary files independently for transparency, but edits to them do not affect the runtime. The next `load()` always re-establishes the canonical state from `project_identity.json`.
- This design prevents manually edited projections from silently overriding the canonical snapshot.

## 6. Snapshot Model

```rust
pub struct ProjectIdentity {
    pub name: String,
    pub description: Option<String>,
    pub languages: Vec<String>,           // sorted
    pub frameworks: Vec<String>,          // sorted
    pub build_system: Option<String>,
    pub package_manager: Option<String>,
    pub testing_framework: Option<String>,
    pub repository_url: Option<String>,
    pub repository_type: Option<String>,
    pub architecture_summary: Option<String>,
    pub known_patterns: Vec<String>,      // sorted
    pub known_modules: Vec<String>,       // sorted
    pub important_files: Vec<String>,     // sorted
    pub engineering_decisions: Vec<EngineeringDecision>, // sorted by id
    pub known_constraints: Vec<String>,  // sorted
    pub current_sprint: Option<String>,
    pub roadmap: Vec<RoadmapItem>,        // sorted by id
    pub recent_milestones: Vec<String>,   // sorted
    pub coding_conventions: Vec<String>,  // sorted
    pub workspace_root: Option<String>,
    pub schema_version: String,
    pub created_at: Option<String>,
    pub updated_at: String,
}
```

**Determinism:** All vector fields are sorted at construction time.
`PartialEq` ignores `created_at` and `updated_at` (inherently non-deterministic).
Serialization is stable and deterministic.

## 7. Update Model

Updates flow through `IdentityChanges`:

```rust
pub struct IdentityChanges {
    pub add_decisions: Vec<EngineeringDecision>,
    pub update_decision_status: Vec<(String, DecisionStatus)>,
    pub add_constraints: Vec<String>,
    pub add_modules: Vec<String>,
    pub add_patterns: Vec<String>,
    pub add_conventions: Vec<String>,
    pub set_sprint: Option<String>,
    pub add_roadmap_items: Vec<RoadmapItem>,
    pub complete_roadmap_item: Option<String>,
    pub add_milestone: Option<String>,
    pub update_architecture_summary: Option<String>,
    pub add_important_files: Vec<String>,
    pub add_languages: Vec<String>,
    pub add_frameworks: Vec<String>,
    pub set_build_system: Option<String>,
    pub set_package_manager: Option<String>,
    pub set_testing_framework: Option<String>,
}
```

`ProjectIdentityRuntime::update(changes)` applies all non-empty changes,
persists the result, and returns `Some(Ok(&identity))` or `Some(Err(...))`.
Empty changes return `None`.

## 8. EngineeringContext Integration

```rust
// EngineeringContextBuilder consumes the snapshot.
let context = EngineeringContextBuilder::new()
    .project(runtime.snapshot())   // ← immutable snapshot
    .task(intent_plan)
    .workspace(workspace_context)
    .build()?;
```

`ProjectIdentityRuntime` owns persistence. `EngineeringContext` only
consumes snapshots. `ProjectIdentity` does NOT know `PromptBuilder`.

## 9. Migration Strategy

Migrations are defined in `migration.rs` as a ordered list of
`Migration { from_version, to_version, apply }` tuples.

To add a future migration:
1. Define the migration function.
2. Add it to the `MIGRATIONS` constant in version order.
3. Update `CURRENT_SCHEMA_VERSION` only after the migration is deployed.

Backward compatibility is guaranteed: any identity with a known schema
version loads successfully. Unknown versions produce a validation error.

## 10. Future Extension Points

These are documented only — not implemented in Sprint 23.

| Extension | Description |
|-----------|-------------|
| Engineering Memory Runtime | Long-term memory across projects |
| Reflection Runtime | Self-reflection on engineering quality |
| Learning Runtime | Pattern learning from completed tasks |
| Task Graph Runtime | Dependency-aware task planning |
| Knowledge Extraction | Automatic identity enrichment from code |
| Architecture Evolution | Track architecture changes over time |

## 11. Constitution Compliance

| Principle | Compliance |
|-----------|-----------|
| Engineering First | Identity captures engineering knowledge, not chat |
| Project Awareness | Remembers architecture, decisions, constraints |
| Deterministic by Default | Sorted vectors, stable serialization, timestamp-ignoring equality |
| Explainability | Diagnostics expose load/save/migration/validation state |
| Provider Agnostic | No provider coupling; pure Rust persistence |
| Core Remains Small | Focused module; no LLM or prompt logic |
| No Feature Creep | No Learning Engine, no Reflection, no Task Graph |

---

## 12. Files

| File | Purpose |
|------|---------|
| `src/project_identity/mod.rs` | Module root and re-exports |
| `src/project_identity/identity.rs` | Core `ProjectIdentity` type |
| `src/project_identity/runtime.rs` | `ProjectIdentityRuntime` + `ProjectIdentityProvider` trait |
| `src/project_identity/storage.rs` | File-based persistence in `.codebro/` |
| `src/project_identity/loader.rs` | Loading from `.codebro/` with migration |
| `src/project_identity/updater.rs` | Applying engineering state changes |
| `src/project_identity/migration.rs` | Schema version migration pipeline |
| `src/project_identity/builder.rs` | Fluent builder with validation |
| `src/project_identity/diagnostics.rs` | Runtime diagnostics tracking |
| `src/project_identity/statistics.rs` | Aggregate statistics |
| `src/project_identity/validation.rs` | Deterministic validation rules |
