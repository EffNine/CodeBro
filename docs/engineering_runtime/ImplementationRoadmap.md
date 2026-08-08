# Implementation Roadmap

**Phase**: P10.5 — Engineering Runtime Design Summit
**Status**: APPROVED TO DESIGN — NO IMPLEMENTATION
**Version**: 1.0.0

---

## 1. Purpose

This roadmap sequences the future implementation of the Engineering Runtime.
**It is not an authorization to implement** — it is the plan the Chief
Architect reviews. Implementation starts only after explicit approval.

---

## 2. Guiding Rules

1. **Graphs before analysis.** Impact Analysis and the Context Compiler
   depend on graphs; graphs depend on the Symbol Registry; the Symbol
   Registry depends on fact ingestion.
2. **Lazy first.** Every step implements the lazy, incremental contract
   (GraphStrategy.md) — never the eager full-build.
3. **Budgets enforced from step one.** Each phase ships with diagnostics so
   PerformanceBudget.md can be verified continuously.
4. **No existing runtime modified.** All work lands in
   `src/engineering_runtime/` + one `mod` line in `src/main.rs`.
5. **Deterministic answers first.** A phase is complete when its engineering
   questions are answerable without an LLM.

---

## 3. Phases

### Phase P10.5.0 — Foundation & Fact Ingestion

**Goal:** runtime skeleton, facts adapter, Symbol Registry.

- `mod.rs`, `types.rs`, `diagnostics.rs`, `facts.rs`, `registry.rs`
- Ingest `ParseResult`/`Symbol`/`Relationship` from `intelligence/index`.
- Ingest workspace change events from Workspace Runtime.
- Symbol Registry with per-file LRU, staleness tracking.

**Exit criteria**
- [ ] `EngineeringRuntime::new` performs zero I/O (< 100 ms construction test)
- [ ] Symbol Registry populates lazily on first query
- [ ] Change event marks affected symbols dirty
- [ ] Diagnostics expose symbol_count, dirty_files, build_ms
- [ ] `runtime_is_lazy_on_construction` test passes

### Phase P10.5.1 — Dependency Graph

**Goal:** file + symbol dependency graph with transitive queries.

- `dependency.rs` — forward/reverse BFS, path finding, fan-in/out
- Incremental connected-component invalidation
- Node-cap + partial-answer degradation

**Exit criteria**
- [ ] `dependents_of` / `dependencies_of` (transitive) deterministic answers
- [ ] One-file change updates only the connected component
- [ ] Cold build < 250 ms, hot query < 5 ms (diagnostics-verified)

### Phase P10.5.2 — Module Graph

**Goal:** module/package topology.

- `module.rs` — modules from build-system facts + import resolution
- Module SCC for circular dependency detection
- Unused module detection (fan-in = 0)

**Exit criteria**
- [ ] "Which modules are affected?" answered without LLM
- [ ] Circular dependency report via SCC
- [ ] Unused module candidates listed

### Phase P10.5.3 — Call Graph (lazy)

**Goal:** call-site analysis, query-scoped.

- `call.rs` — call edges, dead-code candidate scan, rename call sites
- Always evictable; never resident

**Exit criteria**
- [ ] Dead-code candidates without false positives (entry points seeded)
- [ ] Call sites enumerated for rename impact
- [ ] Call Graph never retained beyond query scope

### Phase P10.5.4 — Test Impact Graph

**Goal:** test ↔ code mapping.

- `test_impact.rs` — parse-time test mapping + optional runtime coverage
- Scope LRU caching

**Exit criteria**
- [ ] "Which tests may fail for file X?" answered without LLM
- [ ] Per-scope caching respects the 8 MB budget

### Phase P10.5.5 — Architecture Graph (optional)

**Goal:** boundary/violation analysis.

- `architecture.rs` — component/layer rules, violation scan, component
  dependents

**Exit criteria**
- [ ] Violations reported only when rules are declared
- [ ] "Which services use this component?" answered deterministically

### Phase P10.5.6 — Relationship Resolution & Impact Analysis

**Goal:** the question-answering engine.

- `resolution.rs` — def/ref/use/import/contains lookups
- `impact.rs` — rename/delete/API/test/module impact (ImpactAnalysis.md)

**Exit criteria**
- [ ] All 10 engineering questions answerable without an LLM
- [ ] Pre-flight integration: rename/delete/API changes gate on severity
- [ ] Memoization sound under graph-version hash

### Phase P10.5.7 — Context Compiler

**Goal:** token-efficient fragments for AI Runtime.

- `compiler.rs` — intent mapping, ranking, fragment assembly, budgeting

**Exit criteria**
- [ ] Engineering fragments ≤ 30% of prompt tokens (benchmark)
- [ ] `compile()` < 10 ms
- [ ] Deterministic intents never reach an LLM

### Phase P10.5.8 — Integration & Validation

**Goal:** full wiring, benchmarks, regression.

- Wire into Context Runtime assembler, tools pre-flight, TUI dashboard
- Performance benchmarks vs. PerformanceBudget.md
- Full regression suite

**Exit criteria**
- [ ] Cold startup < 100 ms, idle memory < 128 MB (benchmarked)
- [ ] All P0–P10.4 tests pass
- [ ] Zero clippy warnings; > 80% coverage on new modules

---

## 4. Dependency Graph of Phases

```
P10.5.0 Foundation
    │
    └──► P10.5.1 Dependency
            │
            ├──► P10.5.2 Module
            │        │
            │        └──► P10.5.5 Architecture
            │
            ├──► P10.5.3 Call
            │
            └──► P10.5.4 Test Impact
                     │
                     ▼
              P10.5.6 Resolution + Impact
                     │
                     ▼
              P10.5.7 Context Compiler
                     │
                     ▼
              P10.5.8 Integration & Validation
```

---

## 5. Implementation Order (module-level)

1. `mod.rs`, `types.rs`, `diagnostics.rs`
2. `facts.rs` — ingestion adapters
3. `registry.rs` — Symbol Registry
4. `dependency.rs` — Dependency Graph
5. `module.rs` — Module Graph
6. `call.rs` — Call Graph
7. `test_impact.rs` — Test Impact Graph
8. `architecture.rs` — Architecture Graph
9. `resolution.rs` — Relationship Resolution
10. `impact.rs` — Impact Analyzer
11. `compiler.rs` — Context Compiler
12. `src/main.rs` — single `mod` declaration
13. `tests.rs` — full test suite

---

## 6. Validation Gates

| Gate | Requirements |
|------|--------------|
| P10.5.0 → 1 | lazy construction test passes; diagnostics wired |
| P10.5.1 → 2 | transitive queries deterministic; incremental invalidation works |
| P10.5.2 → 3 | module SCC correct; no false unused-module positives |
| P10.5.3 → 4 | dead-code scan false-positive-free |
| P10.5.4 → 5 | test-impact correct on fixtures |
| P10.5.5 → 6 | violations only under declared rules |
| P10.5.6 → 7 | all 10 questions answered without LLM |
| P10.5.7 → 8 | fragment budget ≤ 30%; deterministic intents skip LLM |
| P10.5.8 → Release | full budget + regression + coverage met |

---

## 7. Success Metrics

| Metric | Target |
|--------|--------|
| Cold startup | < 100 ms |
| Idle memory | < 128 MB |
| Graph resident total | ≤ 92 MB |
| Engineering answers without LLM | 100% of the 10 question types |
| Fragment token share | ≤ 30% of prompt |
| Regression count | 0 |
| Test coverage (new modules) | > 80% |

---

## 8. Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Symbol ingestion mismatch with parser schema | versioned fact contract + validation tests |
| Lazy build too slow on big repos | node caps, partial answers, batch invalidation |
| False dead-code positives | entry-point seeding + registration markers |
| Graph memory bloat | LRU eviction + per-graph budgets + diagnostics |
| Circular graph building | explicit build-order (no graph triggers full construction) |

---

## 9. References

- [Engineering Architecture](./EngineeringArchitecture.md)
- [Graph Strategy](./GraphStrategy.md)
- [Context Compiler](./ContextCompiler.md)
- [Impact Analysis](./ImpactAnalysis.md)
- [Performance Budget](./PerformanceBudget.md)
- [Design Summit Report](./DesignSummitReport.md)

---

*Implementation Roadmap — P10.5 Design Summit — APPROVED TO DESIGN. No implementation until Chief Architect review.*
