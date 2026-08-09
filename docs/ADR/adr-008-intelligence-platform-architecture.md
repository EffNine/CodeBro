# ADR-008: Intelligence Platform Architecture

**ADR Number:** ADR-008
**Title:** Intelligence Platform Architecture
**Author:** CodeBro Engineering
**Status:** Accepted
**Created:** 2026-08-05
**Updated:** 2026-08-05
**Supersedes:** None
**Related RFC:** None

---

## 1. Context

### 1.1 Background

The Foundation v1.0 (P0–P3) established the runtime, reliability, and tool platform for CodeBro. The intelligence layer (`src/intelligence/`) was introduced as a stub in the architecture manifest but has since accumulated partial implementations across 8 submodules. These implementations lack:

1. **Trait abstractions** — No formal traits define the public interface of each component.
2. **Diagnostics** — No observability into parse performance, index health, graph integrity, or search quality.
3. **Architectural contracts** — No formal documentation of component contracts.
4. **Module boundary enforcement** — The intelligence layer currently has no clear integration contract with the runtime pipeline.

P4 aims to solidify the intelligence layer as a proper platform component, not an experimental feature.

### 1.2 Constraints

- Must respect existing architecture manifest boundaries (`intelligence/` is read-only).
- Must not introduce new dependencies beyond `tree-sitter` variants.
- Must maintain `#![allow(dead_code, unused_imports, unused_variables, clippy::all)]` for forward-compatibility.
- Must not implement embedding models, AI memory optimization, or automatic code editing.
- Must be reviewable before P4.5 feature work begins.

### 1.3 Stakeholders

- **Agent Layer**: Will consume context built by the intelligence platform.
- **TUI Layer**: Will display intelligence diagnostics.
- **Future LSP Integration**: Will use the LSP abstraction as a foundation.
- **Research/Planning Agents**: Will use reasoning and search interfaces.

---

## 2. Decision

### 2.1 Decision Statement

The Intelligence Platform adopts a **trait-abstracted, diagnostics-rich, contract-first** architecture. Each of the 10 required components is defined by a formal trait, implemented with a concrete type, and instrumented with diagnostics. Contracts are documented in `docs/contracts/`.

### 2.2 Rationale

1. **Trait abstractions enable swap-in implementations** — The `SymbolDatabase` trait allows future replacement of SQLite with other backends without changing consumers.
2. **Diagnostics enable observability** — Without diagnostics, index build failures and search degradation are invisible.
3. **Contracts enable parallel development** — The agent layer can be built against contracts before the intelligence layer is fully wired in.
4. **Read-only boundary is preserved** — Intelligence remains a read-only analysis layer; it never writes files or executes commands.

### 2.3 Principles Applied

- **Separation of Interface and Implementation** — Every component exposes a trait; consumers depend only on the trait.
- **Observability by Default** — Every public operation records diagnostics.
- **Incremental Extendability** — Extension points are documented; reserved interfaces are marked.
- **Architecture Freeze Compliance** — Changes to intelligence module boundaries require an ADR.

---

## 3. Consequences

### 3.1 Positive Consequences

- Clear public API for each intelligence component.
- Diagnostics provide real-time health monitoring.
- Contracts enable independent testing of each component.
- Extension points are documented for P4.5 and beyond.

### 3.2 Negative Consequences

- Trait indirection adds a small performance overhead (negligible).
- More files to maintain (each module now has a trait definition).
- New developers must understand both trait and implementation layers.

### 3.3 Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| Trait overhead | Virtual dispatch cost | Bounds-checked; critical paths use concrete types |
| Contract density | More documentation to maintain | Contracts are machine-readable (Rust traits) |
| Module count | More files in `intelligence/` | Each module has a clear single responsibility |

### 3.4 Impact on Architecture

| Module | Impact |
|--------|--------|
| `intelligence/parser/` | Exposes `CodeParser` trait; `TreeSitterParser` implements it |
| `intelligence/index/` | Exposes `SymbolDatabase` and `CodeIndexer` traits |
| `intelligence/graph/` | Exposes `DependencyGraph` trait with cycle-safe algorithms |
| `intelligence/search/` | Exposes `SemanticSearch` trait with pluggable scoring |
| `intelligence/context/` | Exposes `ContextBuilder` trait; integrates search + graph |
| `intelligence/reasoning/` | Exposes `ReasoningEngine` trait |
| ~~`intelligence/memory/`~~ | Removed in ADR-012 — project knowledge is owned by `project_identity`; fact model by `engineering_facts` |
| `intelligence/lsp/` | Exposes `LspFoundation` trait; stubs for future LSP server |
| `intelligence/diagnostics/` | **New** — `IntelligenceDiagnostics` trait with parse/index/graph/search/context metrics |

### 3.5 Impact on Future Work

- **P4.5**: Embedding-based search can implement `SemanticSearch` without changing consumers.
- **P4.5**: LSP server can implement `LspFoundation` with full protocol support.
- **P5**: Memory optimization can add a `MemoryOptimizer` trait without touching existing memory.
- **P5**: Incremental parsing can add an `IncrementalParser` trait.

---

## 4. Alternatives Considered

| Alternative | Description | Pros | Cons | Why Rejected |
|-------------|-------------|------|------|--------------|
| A: No traits, structs only | Use concrete types directly | Simpler | No swap-in capability | Violates separation principle |
| B: Monolithic intelligence module | Single module, no sub-module traits | Fewer files | Tight coupling | Violates modularity |
| C: Trait + impl per submodule (chosen) | Formal traits, concrete impls, diagnostics | Extensible, observable, testable | More files | Best balance of concerns |

---

## 5. Implementation Notes

### 5.1 Code Patterns

```rust
// Every component must expose a trait and a concrete implementation.
pub trait CodeParser: Send + Sync {
    fn parse(&mut self, source: &str, file_path: &str) -> Result<ParseResult>;
}

pub struct TreeSitterParser { /* ... */ }
impl CodeParser for TreeSitterParser { /* ... */ }

// Diagnostics must instrument every public operation.
pub struct IntelligenceDiagnostics {
    inner: Arc<Mutex<IntelligenceDiagnosticsInner>>,
}
```

### 5.2 Anti-Patterns

```rust
// DO NOT: Hardcode language support without trait abstraction
fn parse_rust(source: &str) -> Vec<Symbol> { /* ... */ }

// DO: Use the trait
fn parse(&mut self, source: &str, file_path: &str) -> Result<ParseResult>;
```

### 5.3 Migration Steps

1. Define traits in each submodule's `mod.rs`.
2. Refactor existing implementations to satisfy traits.
3. Add `IntelligenceDiagnostics` module.
4. Instrument all public methods with diagnostic recording.
5. Add tests for each trait + implementation pair.
6. Create contracts documentation.
7. Update architecture manifest.

---

## 6. References

- [Architecture Snapshot v1.0](../architecture/architecture_snapshot_v1.md)
- [Architecture Manifest v1.0](../architecture/architecture_manifest_v1.md)
- [SOP v1.0](../SOP/codebro_sop_v1.md)

---

## 7. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-05 | Created | CodeBro Engineering |
