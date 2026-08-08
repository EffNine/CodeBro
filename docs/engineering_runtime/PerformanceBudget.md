# Performance Budget

**Phase**: P10.5 — Engineering Runtime Design Summit
**Status**: APPROVED TO DESIGN — NO IMPLEMENTATION
**Version**: 1.0.0

---

## 1. Budget

| Metric | Target | Enforcement Mechanism |
|--------|--------|----------------------|
| Cold startup | **< 100 ms** | no graph construction at startup; construction does zero I/O |
| Idle memory | **< 128 MB** | per-graph budgets ≤ 92 MB resident total |
| No eager graph construction | ✓ | all graphs Lazy/Optional; only Symbol Registry is Always Available |
| Incremental updates only | ✓ | change-event-driven dirty-set updates; no full rebuilds |
| Query latency (hot graphs) | **< 5 ms** | in-memory structures; O(1) lookups |
| First lazy build (dependency graph) | **< 250 ms** | per-query build budget; partial answers flagged |
| Context fragment generation | **< 10 ms** | ranking + assembly on cached facts |

---

## 2. Where the Budget Goes

### 2.1 Graph Memory Budgets

| Graph | Class | Budget | Notes |
|-------|-------|--------|-------|
| Symbol Registry | Always Available | ≤ 32 MB | LRU per file |
| Dependency Graph | Lazy | ≤ 24 MB | LRU component |
| Module Graph | Lazy | ≤ 8 MB | LRU |
| Call Graph | Lazy | ≤ 16 MB | query-scoped, evict immediately |
| Test Impact Graph | Lazy | ≤ 8 MB | per-scope LRU |
| Architecture Graph | Optional | ≤ 4 MB | rules resident |
| **Total** | | **≤ 92 MB** | under the 128 MB idle cap |

Reserved headroom of ~36 MB for diagnostics, working sets, and transient
build buffers.

### 2.2 Working Buffer Budgets

| Buffer | Budget |
|--------|--------|
| Transitive closure results | bounded by node cap (GraphStrategy §9) |
| Ingestion queue (change events) | bounded FIFO, debounced |
| Context fragment assembly | bounded by caller token budget |

---

## 3. Cold Startup < 100 ms

**Construction contract:**

```
EngineeringRuntime::new(root, fact_source):
    · store root + fact source reference      (no I/O)
    · create empty Symbol Registry stub        (no parse)
    · register graph descriptors               (no construction)
    · wire diagnostics                         (cheap counters)
```

- **No** graph is built.
- **No** filesystem walk (Workspace Runtime owns that).
- **No** parser invocation (intelligence layer owns that).
- **No** SQLite connection beyond what the index already holds.

**Verified by:** a `runtime_is_lazy_on_construction` test that asserts zero
ingestion, zero graph construction, and sub-100 ms wall time — mirroring the
Workspace Runtime's `runtime_is_lazy_on_construction` test.

---

## 4. Idle Memory < 128 MB

1. **Nothing is retained until queried.** The Symbol Registry populates on
   first lookup or first sync event; other graphs stay `Unbuilt`.
2. **LRU eviction everywhere.** Each graph has an eviction policy
   (GraphStrategy §6); evicted structures rebuild on demand.
3. **Call Graph is never resident.** It is built per query and immediately
   evictable — the single largest memory item is bounded to ≤ 16 MB.
4. **No source text retained.** Facts are stored as compact symbol/location
   records, not file contents. Doc comments are stored as hashes unless
   requested.
5. **No duplicate of workspace facts.** File lists and change events are
   referenced from the Workspace Runtime, never copied wholesale.

---

## 5. Incremental Updates

### 5.1 Cost Model

| Operation | Cost |
|-----------|------|
| One file changed | dirty one file's symbols + connected component |
| One file added/removed | add/remove node + edges; recompute affected SCC only |
| Manifest change | mark module graph dirty; cascade to architecture |
| Bulk change (branch switch) | debounced batch; one invalidation pass |

### 5.2 Anti-Patterns (prohibited)

| Anti-pattern | Why |
|--------------|-----|
| Full dependency graph rebuild on any change | O(V+E) every edit — violates incremental-only |
| Re-parsing files for graph updates | duplicate of intelligence layer |
| Retaining source text for doc comments | memory bloat |
| Background graph warming thread | idle CPU + eager construction |

---

## 6. Query Latency Targets

| Query | Target |
|-------|--------|
| `lookup(name, scope)` | < 1 ms |
| `references_of(symbol)` | < 3 ms |
| `dependents_of(file)` (hot) | < 5 ms |
| `dependents_of(file)` (cold, lazy build) | < 250 ms |
| `tests_affected_by(files)` | < 20 ms |
| `architecture.violations()` | < 50 ms |
| `compiler.compile(request)` | < 10 ms |

All latency is measured with `Instant` and surfaced via
`EngineeringDiagnostics` — the budget is auditable at runtime.

---

## 7. Degradation Model

If a workspace is huge or a query explodes, the runtime degrades gracefully:

| Condition | Behavior |
|-----------|----------|
| Lazy build exceeds 250 ms | return `partial` answer with `truncated`/`partial` flag |
| Transitive closure exceeds node cap | truncate result, set `truncated` |
| Graph exceeds memory budget | LRU eviction; rebuild on next query |
| Workspace too large for full indexing | operate on the subset covered by the index; report `coverage` |
| No parser facts available | answer `unindexed` with workspace-only facts |

The runtime never blocks the shell; worst-case latency is always bounded by
the caller's chosen caps.

---

## 8. Diagnostics → Budget Auditability

`EngineeringDiagnostics` exposes:

```
graph_count         — how many graphs materialized
symbol_count        — resident symbols
dirty_files         — files awaiting re-sync
build_ms            — per-graph cumulative build latency
query_ms            — per-graph cumulative query latency
cache_hits          — query cache hit rate
truncations         — number of capped results
coverage            — indexed files / total files
```

These map 1:1 to the budget table in §1, so a dashboard can show compliance
in real time.

---

## 9. Buddy-Check Against Budget

| Requirement | Where held |
|-------------|------------|
| Cold startup < 100 ms | §3 — zero I/O construction + lazy construction test |
| Idle memory < 128 MB | §2.1 — ≤ 92 MB graph budgets + eviction |
| No eager graph construction | §4.1 — nothing retained until queried |
| Incremental updates only | §5 — dirty-set, debounced, no full rebuilds |
| First query within latency | §6 — targets + degradation model |

---

## 10. References

- [Engineering Architecture](./EngineeringArchitecture.md)
- [Graph Strategy](./GraphStrategy.md)
- [Impact Analysis](./ImpactAnalysis.md)
- [Workspace Performance Budget](../WorkspacePerformanceBudget.md)
- [Runtime Architecture v2 §Success Criteria](../summit/RuntimeArchitecture.md)

---

*Performance Budget — P10.5 Design Summit — APPROVED TO DESIGN*
