# Engineering Facts Architecture Report

**Phase**: P10.5.0 — Engineering Facts Model
**Status**: APPROVED TO IMPLEMENT → IMPLEMENTED → Await Chief Architect Review

## 1. Mission

Implement the **Engineering Facts Model**: the canonical engineering data
model consumed by the Engineering Runtime. Facts become the **only public
contract** between language intelligence providers and the Engineering
Runtime. The Engineering Runtime **never consumes source code directly**.

## 2. Architecture Contract

Engineering Facts represent **engineering knowledge** — not syntax, not AST,
not parser output, not compiler internals. Facts are **language-neutral**.

### Engineering Facts own

| Entity | ID type | Type | File |
|--------|---------|------|------|
| Workspace | `WorkspaceId` | `WorkspaceFact` | `package.rs` |
| Package | `PackageId` | `PackageFact` | `package.rs` |
| Module | `ModuleId` | `ModuleFact` | `module.rs` |
| Symbol | `SymbolId` | `SymbolFact` | `symbol.rs` |
| Test | `TestId` | `TestFact` | `test.rs` |
| Build Target | `BuildTargetId` | `BuildTargetFact` | `build_target.rs` |
| Dependency | `DependencyId` | `DependencyFact` | `dependency.rs` |
| Relationship | `RelationshipId` | `RelationshipFact` | `relationship.rs` |
| Reference | `ReferenceId` | `ReferenceFact` | `relationship.rs` |
| Diagnostic | `DiagnosticId` | `DiagnosticFact` | `diagnostics.rs` |
| Architecture Rule | `ArchitectureRuleId` | `ArchitectureRuleFact` | `architecture.rs` |
| Visibility | — | `Visibility` | `visibility.rs` |
| API Surface | — | `ApiSurface` | `symbol.rs` |
| Source Location | — | `SourceLocation` | `location.rs` |
| Metadata | — | `FactMetadata` | `metadata.rs` |
| Ids | — | `WorkspaceId` … `ArchitectureRuleId`, `FactId` | `ids.rs` |

### Engineering Facts do NOT own

Parser, AST, Lexer, Compiler, Runtime, workspace discovery, Git, AI. **None
of these appear anywhere in the module** — confirmed by grep for `parser`,
`ast`, `token`, `syntax`, `compiler`, `tree_sitter`, `uuid`, `chrono`.

## 3. Module Structure

```
src/engineering_facts/
  mod.rs          — FactsModel (immutable aggregate) + FactsBuilder + re-exports
  ids.rs          — strongly-typed opaque IDs (WorkspaceId … ArchitectureRuleId) + FactId union
  types.rs        — FactKind, Severity
  symbol.rs       — SymbolFact, SymbolKind, ApiSurface
  module.rs       — ModuleFact
  package.rs      — PackageFact, WorkspaceFact
  dependency.rs   — DependencyFact, DependencyKind
  relationship.rs — RelationshipKind, RelationshipFact, ReferenceFact
  visibility.rs   — Visibility
  test.rs         — TestFact
  build_target.rs — BuildTargetFact, BuildTargetKind
  location.rs     — SourceLocation, Span, Position
  metadata.rs     — FactMetadata, Tag, Attribute
  diagnostics.rs  — DiagnosticFact
  architecture.rs — ArchitectureRuleFact
  validation.rs   — FactsValidator, ValidationReport, ValidationRule
  tests.rs        — 36 unit/integration tests
```

## 4. Architectural Principles

1. **Facts-only contract** — every public type is pure engineering
   knowledge; nothing can observe or parse source.
2. **Opaque strongly-typed IDs** — one ID type per entity
   (`WorkspaceId`…`ArchitectureRuleId`) plus an `FactId` union for
   cross-entity references. No UUID generation, no timestamps, no
   randomness.
3. **Immutable after creation** — facts are value types (public fields, no
   mutators); `FactsModel` is frozen by `FactsBuilder::build()` and has no
   mutation path.
4. **Deterministic** — every category is stored id-sorted; build,
   serialisation and validation are byte-identical across runs.
5. **Thread-safe** — every type is `Send + Sync`; verified by a
   concurrency test sharing the model through `Arc` across 8 threads.
6. **Zero-heap lookups** — id lookups are binary searches over sorted
   slices (`O(log n)`, no allocation); `FactMetadata::get`/`has_tag` are
   allocation-free partition-point searches.
7. **Serialisable end-to-end** — every entity, the aggregate and the
   validation report round-trip through JSON and TOML.

## 5. Data Flow

```
Language intelligence providers
   │  produce facts (never source)
   ▼
FactsBuilder  (mutable, any order)
   │  build()
   ▼
FactsModel  (immutable, id-sorted, Send + Sync)
   │  workspace()/symbol()/…   binary-search lookups, no alloc
   │  validate()              deterministic rules
   ▼
Engineering Runtime  (consumes facts only; downstream phase)
```

## 6. Design Decisions

- **Strongly-typed closed enums** (`Visibility`, `RelationshipKind`,
  `SymbolKind`, …) with `parse`/`as_str`/`ALL`. Language-specific values
  cannot enter the model; unknown strings map to `None`.
- **`FactId` is a union** — relationship/reference/dependency endpoints and
  diagnostic `related` lists point at any fact kind via `FactId`; entity
  ids convert into it losslessly.
- **Validation is advisory-by-design** — `Unknown` visibility and orphan
  symbols are warnings; `passed()` requires zero error/fatal findings.
- **Ids are globally unique** — duplicate detection flags any opaque id
  that appears more than once across *all* categories.
- **No traits over the entity types** — a private `IdCarrier` helper inside
  the module implements the uniform id-sorting used by the builder.

## 7. Acceptance Criteria Compliance

| Criterion | Status |
|-----------|--------|
| Zero parser implementation | ✅ no parser code anywhere in the module |
| Zero AST | ✅ no AST representation |
| Zero language-specific code | ✅ language-neutral enums only; `language` is an optional string tag |
| Thread-safe | ✅ `Send + Sync`; concurrency test |
| Immutable | ✅ value types + frozen aggregate |
| Fully deterministic | ✅ sorted storage; determinism tests |
| Public API documented | ✅ `//!` module docs + doc comments on every public item |
| Complete unit tests | ✅ 36 tests |
| Zero regressions | ✅ full suite 2072 passed / 0 failed |

## 8. Out of Scope (per session contract)

Graph Store, Relationship Engine, Context Compiler and the Engineering
Runtime are **not** implemented. This phase ships the fact model contract
only.

## 9. Owned / Not-owned Inventory

- **Owned**: `WorkspaceId`, `PackageId`, `ModuleId`, `SymbolId`, `TestId`,
  `BuildTargetId`, `DependencyId`, `RelationshipId`, `DiagnosticId`,
  `ArchitectureRuleId`, `SourceLocation`, `Metadata`.
- **Not owned**: parser, AST, lexer, compiler, runtime, workspace
  discovery, git, AI.
