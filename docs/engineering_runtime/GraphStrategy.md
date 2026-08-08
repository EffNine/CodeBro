# Graph Strategy

**Phase**: P10.5 — Engineering Runtime Design Summit
**Status**: APPROVED TO DESIGN — NO IMPLEMENTATION
**Version**: 1.0.0

---

## 1. Purpose

The Engineering Runtime owns multiple graphs. Every graph must be classified,
and must define its build trigger, invalidation trigger, cache policy, memory
budget, and expected complexity — before any implementation. This document is
the graph strategy contract.

---

## 2. Classification Taxonomy

Every graph carries exactly one availability class:

| Class | Meaning | Construction | Lifespan |
|-------|---------|--------------|----------|
| **Always Available** | Built at first access, maintained incrementally, kept resident | On demand, then kept | Until workspace change / eviction |
| **Lazy** | Built only when queried; may be evicted after use | On demand per query | Evictable |
| **Optional** | Built only if the project satisfies enabling conditions | Conditional | Evictable |
| **Never Cached** | Computed fresh per query; never retained | Every query | Query-scoped |

**Global rules:**

1. No graph is constructed at startup (cold-start budget, §PerformanceBudget).
2. Default class is **Lazy**. `Always Available` requires justification.
3. `Never Cached` is for queries too expensive to retain but where the result
   must always be fresh.
4. Every graph maintains a **dirty set** — nodes whose inputs changed — and
   re-materializes only those nodes.

---

## 3. Graph Registry

| Graph | Class | Build Trigger | Invalidation Trigger | Cache Policy | Memory Budget | Expected Complexity |
|-------|-------|---------------|----------------------|--------------|---------------|---------------------|
| Symbol Registry | Always Available | First symbol/reference query, or first index sync | Workspace change event touching a file; index invalidation | Keep hot; evict least-referenced by file | ≤ 32 MB | O(s) build, O(1) lookup |
| Dependency Graph | Lazy | First dependency query | Workspace change; symbol registry change | Cache until invalidated; evict least-recently-used | ≤ 24 MB | O(V+E) build, O(V+E) transitive |
| Module Graph | Lazy | First module query | Manifest/build-file change; file move | Cache until invalidated | ≤ 8 MB | O(M) build |
| Call Graph | Lazy | First call-graph query (never eagerly) | Any touched file | Query-scoped, always evictable | ≤ 16 MB (bounded) | O(C) build — most expensive |
| Test Impact Graph | Lazy | First test-impact query | Test file change; code change under test | Cache until invalidated; per-scope | ≤ 8 MB | O(T×C) build |
| Architecture Graph | Optional | First architecture query IF rules declared | Rule change; module boundary change | Cache rules; compute per query | ≤ 4 MB | O(M+E) |

**Total worst-case resident memory: ≤ 92 MB** (within the < 128 MB idle
budget — see PerformanceBudget.md).

---

## 4. Per-Graph Strategy

### 4.1 Symbol Registry — Always Available

- **Why Always Available:** every other graph and every relationship query
  depends on it; it is the cheapest structure and the canonical entry point.
- **Build trigger:** first query, or a sync event from the intelligence index.
- **Ingestion:** consumes `ParseResult` / `Symbol` + `Relationship` rows from
  `intelligence/index` (SQLite). No re-parsing in the Engineering Runtime.
- **Invalidation:** workspace `watcher` change event for a file → mark that
  file's symbols dirty; refresh from index.
- **Cache policy:** LRU over files; evict the least-referenced file's symbols
  first when over budget.
- **Complexity:** insertion O(1); lookup O(1) by (name, module); enumerate
  O(s).

### 4.2 Dependency Graph — Lazy

- **Edges:** `imports`, `uses`, `defines`, `overrides`, `implements`, at both
  symbol and file granularity.
- **Build trigger:** first query for dependents/dependencies of any node, or
  a path query.
- **Derived answers (no LLM):**
  - transitive dependents (`dependents_of`),
  - transitive dependencies (`dependencies_of`),
  - shortest path between nodes,
  - fan-in / fan-out counts.
- **Invalidation:** recompute only the connected component(s) containing
  dirty files. Full rebuild is a diagnostic-only escape hatch.
- **Complexity:** BFS/DFS O(V+E); single-node transitive closure O(deg).

### 4.3 Module Graph — Lazy

- **Nodes:** modules and packages derived from build system facts
  (Cargo crate/`mod`, Python package, npm package) + import resolution.
- **Purpose:** module-level impact, unused module detection, module-level
  circular dependency detection.
- **Build trigger:** first module-scoped query.
- **Invalidation:** manifest/build-file change, file moves, module rename.
- **Complexity:** O(M) build; SCC detection O(M+E).

### 4.4 Call Graph — Lazy (most expensive)

- **Nodes:** functions/methods; **edges:** call sites.
- **Build trigger:** explicit call-graph query only — e.g., dead-code
  candidates, rename reachability, call-chain impact.
- **Policy:** built per query, bounded depth/size, immediately evictable.
  Never promoted to `Always Available`.
- **Complexity:** O(C) build where C = call sites; per-query bound enforced
  (see PerformanceBudget.md §4).

### 4.5 Test Impact Graph — Lazy

- **Edges:** test → code symbols covered (parse-time analysis of `#[test]`,
  test files, plus optional runtime coverage traces from the workspace).
- **Build trigger:** first "which tests are affected" query.
- **Invalidation:** any touched code file or test file.
- **Complexity:** O(T×C); per-scope caching keeps the hot test suites
  resident.

### 4.6 Architecture Graph — Optional

- **Enabled only when:** the workspace declares architecture rules (e.g.,
  `docs/architecture/`, `.codebro/architecture.toml`, module boundary
  conventions).
- **Nodes:** components/layers; **edges:** allowed vs. observed dependencies.
- **Purpose:** architecture violation detection; dependency direction audit;
  "which services use this component".
- **Build trigger:** first architecture query **if** rules exist.
- **Invalidation:** rule change or boundary change.
- **Complexity:** O(M+E) build; violation scan O(E).

---

## 5. Invalidation Model

### 5.1 Source of Change Events

```
Workspace Runtime watcher ──► WatchBatch (added/removed/modified files)
        │
        ▼
Engineering Runtime Invalidation Router
        │
        ├──► Symbol Registry  (mark file symbols dirty)
        ├──► Dependency Graph (mark connected component dirty)
        ├──► Module Graph     (mark module + neighbors dirty)
        ├──► Call Graph       (evict scope entirely — never patch)
        ├──► Test Impact Graph(mark tests referencing dirty symbols dirty)
        └──► Architecture Graph (recompute if boundary rules affected)
```

### 5.2 Invalidation Rules

1. **Coarse eviction for cheap graphs, fine-grained for expensive ones.**
   Call Graph evicts wholesale; Symbol Registry updates per file.
2. **Staleness is explicit, not silent.** A query on dirty data returns with
   a `StaleGraph` diagnostic; the caller may request a fresh build.
3. **Batched debounce.** Rapid successive change events are batched into one
   invalidation pass (aligns with Workspace Runtime `watcher.poll`).
4. **Never read files.** Invalidation is driven by events and index facts
   only.

### 5.3 Staleness States

| State | Meaning |
|-------|---------|
| `Clean` | All inputs current; query is authoritative |
| `Dirty` | Some nodes stale; query may be answered with `StaleGraph` flag or trigger build |
| `Evicted` | Structure released from memory; rebuild on demand |
| `Unbuilt` | Never constructed |

---

## 6. Cache Policy Details

| Graph | Retention | Eviction | Coherence |
|-------|-----------|----------|-----------|
| Symbol Registry | Resident | LRU per file | Event-synchronized |
| Dependency Graph | Until invalidation | LRU component | Event-synchronized |
| Module Graph | Until invalidation | LRU | Event-synchronized |
| Call Graph | Query-scoped | Immediate | Always evicted on change |
| Test Impact Graph | Per-scope | LRU scope | Event-synchronized |
| Architecture Graph | Rules resident, results per query | Rules LRU | Recompute on rule change |

---

## 7. Graph Dependencies (build order)

```
Symbol Registry
    │
    ├──► Dependency Graph
    ├──► Call Graph
    └──► Test Impact Graph
Module Graph (independent, built from build-system facts)
    │
    └──► Architecture Graph
```

No graph ever triggers another graph's *full* construction implicitly; a
query may trigger only its direct prerequisites.

---

## 8. "Which graph answers which question"

| Question | Primary Graph | Class |
|----------|---------------|-------|
| What depends on this file? | Dependency Graph | Lazy |
| What depends on this symbol? | Dependency Graph | Lazy |
| Which modules are affected? | Module Graph | Lazy |
| Which tests may fail? | Test Impact Graph | Lazy |
| Which APIs may break? | Symbol Registry (public API) + Dependency Graph | Always Available + Lazy |
| Which services use this component? | Architecture Graph | Optional |
| Is there a circular dependency? | Module/Dependency SCC | Lazy |
| Is this module unused? | Module Graph (fan-in = 0) | Lazy |
| Dead code candidates? | Call Graph (no callers) | Lazy |

---

## 9. Complexity & Budget Guardrails

- Every transitive query is bounded by a node cap; results above the cap are
  truncated with a `truncated` flag (mirrors Workspace `Listing.truncated`).
- A per-query **build budget** prevents a single lazy build from exceeding
  e.g. 250 ms wall-clock; beyond that the graph degrades to a partial answer
  flagged as `partial`.
- Every graph records its build/query latency in `EngineeringDiagnostics` so
  the budgets in PerformanceBudget.md are auditable.

---

## 10. Acceptance Criteria (Design)

| Requirement | Status |
|-------------|--------|
| Every graph classified | ✅ Always Available / Lazy / Optional / Never Cached |
| Every graph defines build trigger | ✅ §3, §4 |
| Every graph defines invalidation trigger | ✅ §3, §5 |
| Every graph defines cache policy | ✅ §3, §6 |
| Every graph defines memory budget | ✅ §3 (≤ 92 MB total) |
| Every graph defines expected complexity | ✅ §3, §4 |
| No eager construction | ✅ all graphs Lazy/Optional except Symbol Registry (justified) |
| Incremental updates only | ✅ §5 invalidation router; dirty-set updates |

---

*Graph Strategy — P10.5 Design Summit — APPROVED TO DESIGN*
