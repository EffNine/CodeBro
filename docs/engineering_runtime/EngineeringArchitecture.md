# Engineering Runtime Architecture

**Phase**: P10.5 — Engineering Runtime Design Summit
**Status**: APPROVED TO DESIGN — NO IMPLEMENTATION
**Version**: 1.0.0

---

## 1. Mission

Design the **Engineering Runtime**: the intelligence layer between the
Workspace Runtime and the AI Runtime.

The Engineering Runtime answers engineering questions **without requiring an
LLM whenever possible**. It is:

- **NOT** a language server.
- **NOT** a compiler.
- **NOT** a git client.
- **IS** an engineering knowledge runtime.

It transforms a static file tree (Workspace Runtime facts) and raw symbol
facts (parser output) into **derived engineering knowledge** — graphs,
relationships, and impact analyses — that answer questions in milliseconds,
deterministically, with zero token spend.

---

## 2. Position in the Runtime Stack

```
┌─────────────────────────────────────────────────────────────┐
│  Layer 5: Integration Pipeline (Intent · Recommendation)     │
├─────────────────────────────────────────────────────────────┤
│  Layer 4: Runtime Core                                       │
│   ┌───────────────────────────────────────────────────────┐  │
│   │  AI Runtime            (P10.0 — token-spending layer) │  │
│   │  Memory Runtime        (P10.0 — knowledge store)      │  │
│   │  Context Runtime       (P10.1 — context assembly)     │  │
│   │  Provider Runtime      (P10.3 — provider routing)     │  │
│   │  Workspace Runtime     (P10.4 — file-tree facts)      │  │
│   │  ENGINEERING RUNTIME   (P10.5 — KNOWLEDGE LAYER)  ◄───┼──┐
│   └───────────────────────────────────────────────────────┘  │
├─────────────────────────────────────────────────────────────┤  │
│  Layer 3: Service Registry / Capability Discovery            │  │
├─────────────────────────────────────────────────────────────┤  │
│  Layer 2: Foundation Engines                                 │  │
│   · Parser (tree-sitter)   · Indexer (SQLite)               │  │
│   · Intelligence layer (symbols, deps)                      │  │
│   · Tools (filesystem, shell, git, patch)                   │  │
├─────────────────────────────────────────────────────────────┤  │
│  Layer 1: Cross-Cutting (observability, reliability, ...)    │  │
└─────────────────────────────────────────────────────────────┘
```

The Engineering Runtime sits **above** the Workspace Runtime and the raw
intelligence/parser layer, and **below** the AI Runtime. It is the layer that
answers engineering questions deterministically, so the AI Runtime only
receives questions that genuinely need probabilistic reasoning.

### Consumers

| Consumer | How It Uses Engineering Runtime |
|----------|--------------------------------|
| AI Runtime | Receives token-efficient context fragments already enriched with engineering facts |
| Context Compiler | Queries graphs to assemble minimal, relevant context |
| Agents (research/planning/coding/testing) | Ask engineering questions directly |
| TUI dashboard | Renders impact/architecture diagnostics |
| Tools (`edit_file`, `patch`) | Pre-flight rename/delete/impact checks |
| Integration Pipeline | Feeds impact analysis into plan generation |

---

## 3. Ownership Contract

### 3.1 The Engineering Runtime owns

| Capability | Module (proposed) | Description |
|-----------|-------------------|-------------|
| Symbol Registry | `registry.rs` | Canonical, deduplicated symbol entities and locations |
| Dependency Graph | `dependency.rs` | Symbol-level and file-level edges |
| Module Graph | `module.rs` | Modules, packages, import/export topology |
| Call Graph (lazy) | `call.rs` | Call sites and callee edges — built only on demand |
| Test Impact Graph | `test_impact.rs` | Test → code coverage mapping, affected-test answers |
| Architecture Graph | `architecture.rs` | Layered/component boundaries + violation detection |
| Engineering Diagnostics | `diagnostics.rs` | Query latencies, graph staleness, coverage |
| Context Compiler | `compiler.rs` | Token-efficient fragment assembly (see ContextCompiler.md) |
| Impact Analysis | `impact.rs` | Rename/delete/API/test/module impact computation (see ImpactAnalysis.md) |
| Relationship Resolution | `resolution.rs` | Definition/usage/reference/containment lookups |

### 3.2 The Engineering Runtime does NOT own

```
❌ Filesystem            → Workspace Runtime (filesystem.rs)
❌ Git                   → Workspace Runtime observes; git client is external
❌ Provider              → Provider Runtime
❌ Memory                → Memory Runtime
❌ AI                    → AI Runtime
❌ Workspace discovery   → Workspace Runtime (discovery.rs)
❌ LSP implementation    → language servers (Engineering Runtime may CONSUME LSP facts)
❌ Compiler              → build tooling / external compiler
❌ Execution             → tools/executor
❌ Parsing               → intelligence/parser (tree-sitter) — consumed as facts
```

The Engineering Runtime **consumes** parser/symbol facts from the
intelligence layer and workspace facts from the Workspace Runtime. It never
reads source files directly for graph construction; it ingests pre-parsed
facts. This keeps it deterministic, fast, and decoupled from file I/O.

---

## 4. Architectural Principles

1. **Answer without tokens.** Deterministic graph queries are the default
   answer path. LLM is a fallback, never the primary path.
2. **Lazy by default.** No graph is constructed at startup. Every graph is
   built on first query and maintained incrementally.
3. **Incremental only.** On file change, only affected subgraphs are updated.
   Full rebuilds are prohibited by default.
4. **Fact-based ingestion.** Consumes parsed symbols and workspace facts;
   does not re-parse or re-walk the filesystem.
5. **Immutable results.** Every query returns an immutable, cloneable result
   (following the Workspace Runtime pattern).
6. **Observable.** Every query emits latency + staleness telemetry through
   `EngineeringDiagnostics`.
7. **Deterministic before AI.** Rule-based answers win over probabilistic
   ones (project principle #9).
8. **Boundary-respecting.** Owns knowledge, never I/O, never execution.
9. **In-memory fast path, persistent index.** Hot graphs live in memory;
   canonical symbol data persists in the SQLite index (existing
   `intelligence/index/database.rs`).

---

## 5. High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      Engineering Runtime                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ┌───────────────────────────────────────────────────────────┐   │
│  │                    Symbol Registry                        │   │
│  │  (canonical entities, locations, visibility, public API)  │   │
│  └──────────────────────┬────────────────────────────────────┘   │
│                         │ ingestion                              │
│  ┌──────────────────────▼────────────────────────────────────┐   │
│  │                 Graph Store (in-memory)                   │   │
│  │                                                           │   │
│  │   Dependency   Module    Call      Test-Impact  Arch      │   │
│  │   Graph        Graph     Graph     Graph       Graph      │   │
│  │                                                           │   │
│  └──────────────────────┬────────────────────────────────────┘   │
│                         │ queries                                 │
│  ┌──────────────────────▼────────────────────────────────────┐   │
│  │              Relationship Resolution                      │   │
│  │  (definition · references · imports · contains · calls)   │   │
│  └───────────────┬───────────────────┬───────────────────────┘   │
│                  │                   │                           │
│  ┌───────────────▼──────┐  ┌─────────▼───────────────────────┐   │
│  │   Impact Analysis    │  │       Context Compiler          │   │
│  │  (rename, delete,    │  │  (token-efficient fragments)    │   │
│  │   API, test, module) │  │                                 │   │
│  └───────────────┬──────┘  └─────────┬───────────────────────┘   │
│                  │                   │                           │
│  ┌───────────────▼───────────────────▼───────────────────────┐   │
│  │              Engineering Diagnostics                      │   │
│  │  (query latency · graph staleness · coverage · counters)  │   │
│  └───────────────────────────────────────────────────────────┘   │
│                                                                   │
│  Inputs:  workspace facts (WorkspaceRuntime) + symbol facts       │
│           (intelligence/parser + index)                           │
│  Outputs: deterministic answers + enriched context fragments      │
│                                                                   │
└─────────────────────────────────────────────────────────────────┘
```

---

## 6. Data Flow

### 6.1 Ingestion (bottom-up)

```
intelligence/parser  ── ParseResult ──►  Symbol Registry
intelligence/index   ── Symbol + Relationship rows ──►  Symbol Registry
Workspace Runtime    ── file list, changes, build system ──►  Graph Store
```

Ingestion is **event-driven**: workspace change events (from Workspace
Runtime `watcher`) and index invalidation events trigger incremental updates.

### 6.2 Query (top-down)

```
Question (e.g. "what depends on file X?")
    │
    ▼
Relationship Resolution ──► Dependency Graph ──► answer (no LLM)
    │
    ▼
Impact Analysis ──► "X dependents, affected tests, API breakage"
    │
    ▼
Context Compiler ──► minimal fragment for AI Runtime (only if LLM needed)
```

---

## 7. Component Specifications

### 7.1 Symbol Registry (`registry.rs`)

- **Purpose:** single source of truth for symbols.
- **Content:** name, kind, visibility, public/private API marker, module
  path, file, line range, doc comment hash, language.
- **Storage:** hot in-memory map + persisted SQLite rows (existing
  `code_index.db` schema reused).
- **Deduplication:** same (file, name, kind, span) ⇒ one entity; merged on
  re-parse.
- **Public API marking:** symbols reachable across module boundaries are
  marked `public`; this feeds "Public API impact".

### 7.2 Dependency Graph (`dependency.rs`)

- **Nodes:** files + symbols.
- **Edges:** `uses`, `imports`, `defines`, `overrides`, `implements`.
- **Derived:** transitive dependents, transitive dependencies, path finding.
- **Classification:** Lazy (see GraphStrategy.md).

### 7.3 Module Graph (`module.rs`)

- **Nodes:** modules/packages (by build system: Cargo crate + `mod`,
  Python package, npm package).
- **Edges:** module import/export, workspace dependency.
- **Purpose:** "which modules are affected", unused module detection,
  circular dependency detection at module granularity.

### 7.4 Call Graph (`call.rs`)

- **Nodes:** functions/methods; **Edges:** calls.
- **Classification:** Lazy — the most expensive graph; built only on
  explicit call-graph queries and never cached eagerly.
- **Purpose:** dead-code candidates, rename reachability, call-site impact.

### 7.5 Test Impact Graph (`test_impact.rs`)

- **Edges:** test → symbols under test (from `#[test]`/test file analysis and
  test execution traces when available).
- **Purpose:** "which tests may fail when file X changes".
- **Classification:** Lazy, with an optional "Never Cached" query mode.

### 7.6 Architecture Graph (`architecture.rs`)

- **Nodes:** components/layers; **Edges:** allowed/observed dependencies.
- **Rules:** declared boundaries (e.g., `intelligence/` must not import
  `tools/`).
- **Purpose:** architecture violation detection, dependency direction audit.
- **Classification:** Optional (only when the project declares architecture
  rules).

### 7.7 Engineering Diagnostics (`diagnostics.rs`)

- Records: query counts, per-graph build latency, staleness (dirty node
  count), coverage (fraction of files with parsed symbols), cache hit rates.
- Mirrors the `WorkspaceDiagnostics` pattern — cheap counters + summaries.

---

## 8. Core Trait Contracts (design sketches)

```rust
pub trait SymbolRegistry: Send + Sync {
    fn lookup(&self, name: &str, scope: &Scope) -> Result<Vec<SymbolEntity>>;
    fn public_api(&self, module: &ModulePath) -> Result<Vec<SymbolEntity>>;
    fn is_stale(&self) -> bool;
}

pub trait GraphStore: Send + Sync {
    fn dependents_of(&self, file: &Path) -> Result<Vec<PathBuf>>;   // transitive
    fn dependencies_of(&self, file: &Path) -> Result<Vec<PathBuf>>;  // transitive
    fn path_between(&self, from: &Path, to: &Path) -> Result<Option<Vec<PathBuf>>>;
    fn mark_stale(&self, files: &[PathBuf]);
}

pub trait TestImpactGraph: Send + Sync {
    fn tests_affected_by(&self, files: &[PathBuf]) -> Result<Vec<TestTarget>>;
}

pub trait ArchitectureGraph: Send + Sync {
    fn violations(&self) -> Result<Vec<ArchitectureViolation>>;
    fn component_of(&self, file: &Path) -> Result<Option<ComponentId>>;
}

pub trait RelationshipResolver: Send + Sync {
    fn references_of(&self, symbol: &SymbolId) -> Result<Vec<Reference>>;
    fn definitions_of(&self, name: &str) -> Result<Vec<SymbolEntity>>;
    fn resolve(&self, q: &RelationshipQuery) -> Result<RelationshipSet>;
}

pub trait ImpactAnalyzer: Send + Sync {
    fn rename(&self, symbol: &SymbolId, to: &str) -> Result<RenameImpact>;
    fn delete(&self, file: &Path) -> Result<DeleteImpact>;
    fn public_api(&self, module: &ModulePath) -> Result<PublicApiImpact>;
    fn tests(&self, files: &[PathBuf]) -> Result<TestImpact>;
}

pub trait ContextCompiler: Send + Sync {
    fn compile(&self, request: &ContextRequest) -> Result<ContextFragment>;
}
```

Consumers depend on these traits only (Runtime Principles §2.4 — trait-based
interchangeability).

---

## 9. Error Model

| Error | Meaning |
|-------|---------|
| `SymbolNotFound` | Lookup miss; query is answered with an empty result, not an error |
| `GraphNotBuilt` | A lazy graph hasn't been built; trigger build transparently |
| `IngestionFailed` | Parser/index facts malformed; log, retry on next event |
| `StaleGraph` | Query on dirty data; caller decides fresh-build vs. accept staleness |
| `NoArchitectureRules` | Architecture queries with no declared boundaries return empty |

Errors are typed (`thiserror`) and never panic; every public query returns
`Result` (Failure as First-Class, §2.3).

---

## 10. Interaction with Sibling Runtimes

| Runtime | Relationship |
|---------|--------------|
| Workspace Runtime | Input: file list, change events, build system facts. Engineering Runtime never walks the FS. |
| AI Runtime | Output: enriched, token-budgeted context; answers that avoid LLM calls entirely. |
| Context Runtime | Engineering Runtime's Context Compiler may be invoked *by* the Context Runtime's assembler. Ownership of assembly stays with Context Runtime; engineering facts are one input source. |
| Memory Runtime | Read: persistent project knowledge (architectural patterns, past impact reports). Engineering Runtime does not write memory. |
| Provider Runtime | Not used directly. |

---

## 11. Module Layout (proposed)

```
src/engineering_runtime/
  mod.rs          — EngineeringRuntime facade + re-exports
  registry.rs     — SymbolRegistry (in-memory + index-backed)
  dependency.rs   — DependencyGraph (symbol + file level)
  module.rs       — ModuleGraph (modules/packages)
  call.rs         — CallGraph (lazy, on-demand build)
  test_impact.rs  — TestImpactGraph (test → code mapping)
  architecture.rs — ArchitectureGraph + violation rules
  resolution.rs   — RelationshipResolver (def/ref/use/import)
  impact.rs       — ImpactAnalyzer (rename/delete/api/test/module)
  compiler.rs     — ContextCompiler (fragment assembly)
  diagnostics.rs  — EngineeringDiagnostics + summaries
  facts.rs        — ingestion adapters (parser/index/workspace → registry)
  types.rs        — entity ids, scopes, errors, results
  tests.rs        — unit + integration tests
```

Only one `mod` declaration is added in `src/main.rs`; no existing runtime is
modified.

---

## 12. Acceptance Criteria Compliance (Design)

| Criterion | Status |
|-----------|--------|
| No code / no runtime implementation | ✅ this document is architecture only |
| No parser / no AST / no graph construction | ✅ design specifies ingestion contracts only |
| Answers without LLM | ✅ deterministic graph queries are the default path |
| Lazy by default | ✅ every graph has explicit build/invalidation policy (GraphStrategy.md) |
| Incremental updates only | ✅ change-event-driven subgraph updates |
| Performance budget | ✅ < 100 ms cold start, < 128 MB idle (PerformanceBudget.md) |
| Ownership respected | ✅ owns knowledge, not FS/git/provider/memory/AI |
| Reports generated | ✅ 7 summit deliverables |

---

## 13. References

- [Graph Strategy](./GraphStrategy.md)
- [Context Compiler](./ContextCompiler.md)
- [Impact Analysis](./ImpactAnalysis.md)
- [Performance Budget](./PerformanceBudget.md)
- [Implementation Roadmap](./ImplementationRoadmap.md)
- [Design Summit Report](./DesignSummitReport.md)
- [Runtime Architecture v2](../summit/RuntimeArchitecture.md)
- [Runtime Layers v2](../summit/RuntimeLayers.md)
- [Runtime Principles v2](../summit/RuntimePrinciples.md)
- [Workspace Architecture Report](../WorkspaceArchitectureReport.md)
- [Architecture Manifest v1](../architecture/architecture_manifest_v1.md)

---

*Engineering Runtime Architecture — P10.5 Design Summit — APPROVED TO DESIGN*
