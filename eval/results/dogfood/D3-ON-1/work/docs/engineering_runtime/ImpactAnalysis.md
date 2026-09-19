# Impact Analysis

**Phase**: P10.5 — Engineering Runtime Design Summit
**Status**: APPROVED TO DESIGN — NO IMPLEMENTATION
**Version**: 1.0.0

---

## 1. Purpose

Impact Analysis is the deterministic engine that answers the engineering
questions CodeBro must answer **without an LLM**. It computes who and what is
affected by a proposed change, from graphs — in milliseconds.

---

## 2. Answer Matrix

| Engineering Question | Primary Data | Algorithm | Deterministic? |
|----------------------|--------------|-----------|----------------|
| What depends on this file? | Dependency Graph | transitive dependents (reverse BFS) | ✅ |
| What depends on this symbol? | Dependency Graph + Call Graph | reverse reference/use closure | ✅ |
| Which modules are affected? | Module Graph | module reachability | ✅ |
| Which tests may fail? | Test Impact Graph | test ↔ symbol mapping | ✅ |
| Which APIs may break? | Symbol Registry (public API) + Dependency Graph | public-surface + external dependents | ✅ |
| Which services use this component? | Architecture Graph | component dependents | ✅ |
| Rename impact | Symbol Registry + Dependency + Call | references + call sites + API surface | ✅ |
| Delete impact | Dependency Graph | transitive dependents + tests + docs refs | ✅ |
| Architecture violations | Architecture Graph | rule × observed-edge mismatch | ✅ |
| Circular dependency | Dependency/Module SCC | Tarjan/iterative SCC | ✅ |
| Unused module | Module Graph | fan-in = 0 | ✅ |
| Dead code candidates | Call Graph | no callers (bottom-up) | ✅ |

Every row has a deterministic path. Probabilistic reasoning (LLM) is only a
post-processing layer on top of these facts, never a substitute.

---

## 3. Core Impact Types

### 3.1 Rename Impact

```
RenameImpact {
  symbol: SymbolId
  new_name: String
  files: Vec<FilePath>          // files containing references
  references: Vec<Reference>    // each call/reference site
  call_sites: Vec<CallSite>     // from Call Graph when built
  public_api: bool              // symbol is part of public API
  external_consumers: usize     // dependents outside the module
  fix_required: Vec<Location>   // sites that MUST change
  auto_fixable: Vec<Location>   // sites that can be updated mechanically
}
```

**Algorithm:**
1. Resolve the symbol to a canonical `SymbolId`.
2. Walk `references_of(symbol)` from the Symbol Registry.
3. Add call sites from the Call Graph (lazy build on demand).
4. If the symbol is public (crosses module boundary), enumerate
   `external_consumers` and mark breaking.
5. Classify each site: in-file, in-module, external.

**Answer:** "Rename Order::total → amount: 6 files, 14 references,
public API break (3 external)." — zero tokens.

### 3.2 Delete Impact

```
DeleteImpact {
  path: FilePath
  direct_dependents: Vec<FilePath>
  transitive_dependents: Vec<FilePath>   // reverse BFS closure
  affected_tests: Vec<TestTarget>
  affected_modules: Vec<ModuleId>
  doc_or_manifest_refs: Vec<Location>    // non-code references
  severity: Severity                     // derived from scope
}
```

**Algorithm:** transitive dependents via reverse BFS on the Dependency Graph;
map to tests via Test Impact Graph; map to modules via Module Graph.

### 3.3 Public API Impact

```
PublicApiImpact {
  module: ModulePath
  api_symbols: Vec<SymbolEntity>         // public surface
  breaking: Vec<BreakingChange>          // signature/visibility removals
  affected_consumers: usize
  affected_tests: Vec<TestTarget>
}
```

**Algorithm:** enumerate the module's public API from the Symbol Registry;
diff against consumers from the Dependency Graph.

### 3.4 Dependency Impact

```
DependencyImpact {
  file: FilePath
  dependencies: Vec<FilePath>            // transitive
  dependents: Vec<FilePath>              // transitive
  circular_paths: Vec<Path>              // if part of a cycle
  build_scope: BuildScope                // which modules rebuild
}
```

### 3.5 Test Impact

```
TestImpact {
  files: Vec<FilePath>                   // changed files
  affected_tests: Vec<TestTarget>        // tests touching those symbols
  expected_failures: Vec<TestTarget>     // direct dependents' tests
  coverage_gaps: Vec<SymbolId>           // changed symbols w/o tests
}
```

**Algorithm:** for each changed file, resolve its symbols, then map through
the Test Impact Graph's `test → symbol` edges.

### 3.6 Module Impact

```
ModuleImpact {
  module: ModuleId
  affected: Vec<ModuleId>                // transitive dependents at module level
  unused: bool                           // fan-in == 0
  circular_with: Vec<ModuleId>           // SCC members
}
```

### 3.7 Architecture Violation

```
ArchitectureViolation {
  from: ComponentId
  to: ComponentId
  rule: ArcRule                        // e.g. "api must not import storage"
  observed_edges: Vec<Edge>
  severity: Severity
}
```

**Algorithm:** for each observed dependency edge, check against allowed edges;
report violations with the rule that was broken.

### 3.8 Circular Dependency

```
CircularDependency {
  cycle: Vec<ModuleId | FilePath>       // members of the SCC
  entry_edge: Edge
  severity: Severity
}
```

**Algorithm:** SCC (Tarjan or iterative Kosaraju) over the Module/Dependency
Graph; any SCC of size > 1 with a return edge is a cycle.

### 3.9 Unused Module

```
UnusedModule { module: ModuleId, inbound_imports: usize /* = 0 */ }
```

### 3.10 Dead Code Candidate

```
DeadCodeCandidate {
  symbol: SymbolId
  callers: usize                        /* = 0 */
  reachability: UnreachableFromEntryPoint
}
```

**Algorithm:** bottom-up on the Call Graph; a function with no callers is a
candidate. Entry points (main, exported API, tests, proc-macro/registration
markers) are seeded to avoid false positives.

---

## 4. Common Algorithms (all deterministic, no LLM)

| Algorithm | Use |
|-----------|-----|
| Reverse BFS / DFS | transitive dependents |
| Forward BFS / DFS | transitive dependencies, reachability |
| Tarjan / iterative SCC | circular dependencies |
| Topological sort | build scope / module ordering |
| Fan-in / fan-out counting | unused modules, hot-spot detection |
| Bottom-up caller scan | dead-code candidates |
| Set difference | public API diff |
| Rule × edge scan | architecture violations |

All are pure graph computations — O(V+E) worst case, bounded by the node cap
from GraphStrategy.md §9.

---

## 5. Severity Model

| Severity | Meaning | Threshold |
|----------|---------|-----------|
| `Info` | informative scope | 0 breaking, small local change |
| `Warning` | non-breaking but broad | > N files, tests, or modules |
| `Breaking` | public API / compile-time break | any external consumer |
| `Critical` | cascade | breaking + circular + tests fail |

Severity is a deterministic function of the impact result, so callers can
gate destructive operations (e.g., block a delete on `Breaking`).

---

## 6. Consumption Points

| Consumer | Use of Impact Analysis |
|----------|------------------------|
| `edit_file` / `patch` tool | pre-flight check before applying change |
| Planning Agent | risk assessment per plan step |
| TUI | inline "this change affects X" preview |
| Integration Pipeline | plan generation input |
| AI Runtime | receives impact summary so the LLM reasons from facts |

---

## 7. Pre-Flight Integration (tools)

When a tool proposes a rename/delete/API change, the flow is:

```
proposed change
    │
    ▼
ImpactAnalyzer.compute(change)
    │
    ├── result = Info/Warning ──► proceed (optionally with notice)
    ├── result = Breaking ──────► require approval (PermissionManager)
    └── result = Critical ──────► block until resolved
```

This reuses the existing `ChangePlan` propose → approve workflow
(`tools/change.rs`) with impact facts attached.

---

## 8. Caching of Impact Results

| Impact type | Cache? | Policy |
|-------------|--------|--------|
| Rename/Delete/Public API | Per-query | Never cached (query-scoped) |
| Test Impact | Scope LRU | Cache per file set |
| Architecture violations | Recompute on rule/edge change | Cached otherwise |
| Circular deps | Cache with dependency graph | Evict on change |

Deterministic results may be memoized within a session by
`(change, graph_version)`; the graph version hash makes cache invalidation
sound.

---

## 9. Edge Cases & Guarantees

1. **Missing symbols:** return empty impact (not an error) — mirrors the
   `SymbolNotFound` error model in EngineeringArchitecture.md §9.
2. **Stale graphs:** results carry a `stale` flag; caller may request a fresh
   build before acting on `Breaking`/`Critical`.
3. **Bounded output:** transitive closures are node-capped; truncation is
   flagged, never silent.
4. **False-positive guards (dead code):** entry points and registration
   patterns are seeded, so exported/macro-registered symbols are never
   flagged.
5. **Language-agnostic:** graphs are language-agnostic; only ingestion
   (parser) is language-specific — the analysis algorithms never are.

---

## 10. Diagnostics

`EngineeringDiagnostics` records per-impact-type:
- query latency, result size, truncation count, stale-hit count.

These surface in the metrics panel and keep the deterministic guarantees
auditable against the Performance Budget.

---

## 11. Acceptance Criteria (Design)

| Engineering Question | Deterministic Answer Designed | Status |
|----------------------|-------------------------------|--------|
| Rename impact | ✅ §3.1 | |
| Delete impact | ✅ §3.2 | |
| Public API impact | ✅ §3.3 | |
| Dependency impact | ✅ §3.4 | |
| Test impact | ✅ §3.5 | |
| Module impact | ✅ §3.6 | |
| Architecture violations | ✅ §3.7 | |
| Circular dependency | ✅ §3.8 | |
| Unused module | ✅ §3.9 | |
| Dead code candidates | ✅ §3.10 | |
| No LLM required | ✅ §4 (pure graph algorithms) | |

---

## 12. References

- [Engineering Architecture](./EngineeringArchitecture.md)
- [Graph Strategy](./GraphStrategy.md)
- [Context Compiler](./ContextCompiler.md)
- [Performance Budget](./PerformanceBudget.md)
- [Architecture Manifest v1 §11 Intelligence](../architecture/architecture_manifest_v1.md)

---

*Impact Analysis — P10.5 Design Summit — APPROVED TO DESIGN*
