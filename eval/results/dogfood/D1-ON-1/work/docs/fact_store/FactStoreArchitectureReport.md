# Fact Store Architecture Report

**Phase**: P10.5.1 — Fact Store Foundation
**Status**: IMPLEMENTED → Await Chief Architect Review

## 1. Mission

Implement the **Fact Store**: the canonical immutable repository for
Engineering Facts. It provides storage, indexing, lookup and validation
boundaries for the frozen `FactsModel` produced by P10.5.0. The store never
parses source, never walks a graph and never performs analysis.

## 2. Architecture Contract

### Fact Store owns

| Boundary | Type | File |
|----------|------|------|
| FactCollection | immutable fact repository | `collection.rs` |
| FactIndex | deterministic, read-only indexes | `index.rs` |
| FactLookup | O(log n) id lookup surface | `lookup.rs` |
| FactQuery | id/kind/scope queries, enumerate, filter | `query.rs` |
| FactValidation | duplicate facts, broken indexes, missing ids, orphan records, schema consistency | `validation.rs` |
| FactSnapshot | immutable, byte-identical snapshots | `snapshot.rs` |
| FactStatistics | deterministic counts and sizes | `statistics.rs` |
| FactDiagnostics | store-level diagnostics summary | `diagnostics.rs` |
| FactStore / FactStoreBuilder | construction + aggregate | `store.rs` |

### Fact Store does NOT own

Graph construction, relationship traversal, impact analysis, context
compilation, parsing, workspace discovery. Confirmed: no parser, AST, graph or
analysis code exists anywhere in the module.

## 3. Module Structure

```
src/fact_store/
  mod.rs          — module docs, component declarations, re-exports
  store.rs        — FactStore (immutable aggregate) + FactStoreBuilder
  collection.rs   — FactCollection (frozen facts repository)
  index.rs        — FactIndex, primary + reverse indexes, FactIdPair,
                    ReverseIndex
  lookup.rs       — FactLookup (borrowed lookup surface)
  query.rs        — FactQuery (id/kind/scope queries, enumerate, filter)
  snapshot.rs     — FactSnapshot (canonical bytes + deterministic digest)
  statistics.rs   — FactStatistics, IndexSizes, ReverseIndexSizes
  diagnostics.rs  — FactDiagnostics + FactDiagnosticsSummary
  validation.rs   — FactValidation, 5 deterministic rules
  tests.rs        — 39 unit/integration/concurrency/scale tests
```

One-line change outside the module: `mod fact_store;` in `src/main.rs`.

## 4. Architectural Principles

1. **Immutable after construction** — facts are inserted only during
   construction (via `FactStoreBuilder` or `FactStore::build(model)`); after
   `build()` the store has no mutation path.
2. **Deterministic** — all storage is id-sorted; snapshots, statistics,
   diagnostics and validation reports are byte-identical for identical inputs.
   No UUIDs, no timestamps, no randomness.
3. **Thread-safe** — every public type is `Send + Sync`; the store is shared
   behind `Arc` across 8 threads in a dedicated concurrency test.
4. **Indexed, not traversed** — `FactIdPair` projections over declared field
   references; reverse indexes map workspace/package/module/symbol owners to
   sorted member sets. No graph traversal, no transitive closure.
5. **Zero heap upsets** — id lookups are binary searches returning `&T` or `&[FactId]`;
   enumeration is an allocation-free slice walk.

## 5. Component Responsibilities

- **FactCollection** wraps the frozen `FactsModel` and exposes typed slices,
  `find`/`contains`, counts and the allocation-free `iter()` enumerator.
- **FactStore** owns one `FactCollection` + one `FactIndex` and derives lookup,
  query, statistics, diagnostics, validation and snapshot surfaces.
- **FactLookup** answers union/typed id lookups plus index membership and
  reverse-scope lookups.
- **FactQuery** answers find-by-id/kind/workspace/package/module/symbol,
  `enumerate()` and sorted `filter()`.
- **FactValidation** runs the five deterministic store rules.
- **FactSnapshot** serialises the canonical JSON of the facts and a fixed
  FNV-1a 64-bit content digest.

## 6. Acceptance Criteria Compliance

| Criterion | Status |
|-----------|--------|
| Immutable | ✅ value types + frozen aggregate |
| Deterministic | ✅ sorted storage; determinism tests on indexes, snapshots, statistics, validation |
| Thread-safe | ✅ `Send + Sync`; 8-thread concurrency test |
| Zero graph logic | ✅ no adjacency/traversal code |
| Zero parser logic | ✅ no parser/AST/compiler references |
| Zero runtime mutation | ✅ no mutation path after construction |
| Public API documented | ✅ module `//!` + doc comments on every public item |
| Complete tests | ✅ 39 tests |
| Zero regressions | ✅ full suite 2111 passed / 0 failed |

## 7. Out of Scope (per session contract)

Graph Store, Relationship Engine and Context Compiler are **not**
implemented. Nothing outside `src/fact_store/` was modified except the
existing module declaration in `src/main.rs`.

## 8. Owned / Not-owned Inventory

- **Owned**: `FactStore`, `FactStoreBuilder`, `FactCollection`, `FactIndex`,
  `ReverseIndex`, `FactLookup`, `FactQuery`, `FactSnapshot`,
  `FactStatistics`, `FactDiagnostics`, `FactValidation`.
- **Not owned**: graph store, relationship engine, context compiler, parser,
  workspace discovery.