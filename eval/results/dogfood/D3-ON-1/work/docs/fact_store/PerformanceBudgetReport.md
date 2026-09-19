# Fact Store Performance Budget Report

**Phase**: P10.5.1 — Fact Store Foundation
**Status**: IMPLEMENTED

> This is the store-level performance report (P10.5.1). The P10.5.0
> model-level report remains at `docs/PerformanceBudgetReport.md`.

## 1. Budget Statement

| Requirement | Design | Verified |
|-------------|--------|----------|
| Construction optimized for bulk loading | sorted-`Vec` absorption; one O(n log n) model sort + one O(n log n) index build; no per-fact indirection | `half_million_fact_scale_smoke` builds a 500 000-fact store in the ~16 s debug suite |
| Lookup O(log n) or better | id lookups are binary searches over sorted slices; reverse indexes use partition-point range scans | head/middle/tail + negative lookups at scale |
| Enumeration allocation-free where practical | category slices chained into a single lazy iterator; `filter` materialises and sorts | `collection_enumerates_all_facts`, `query_enumerate_matches_collection` |
| Support millions of facts | dense `Vec<T>` storage per category + one id-sorted pair list per reverse dimension | 500 000-fact smoke |
| Immutable after build | value types + frozen aggregate; no mutators | compile-time; determinism tests |
| Deterministic output | all sorted storage; no timestamps/randomness | snapshot/statistics/validation determinism tests |

## 2. Lookup Path

```
store.lookup().symbol(&SymbolId)       → binary search over sorted symbols
store.lookup().contains(&FactId)       → dispatch on kind → one binary search
store.lookup().contains_in_kind(kind…)→ binary search over sorted id list
store.lookup().facts_in_symbol(&SymbolId) → partition point → &[FactIdPair]
store.collection().iter()              → chained slice iterator (no allocation)
store.query().filter(pred)             → collect + sort + dedup (lenient only)
```

All simple lookups allocate nothing and borrow the store.

## 3. Memory Model

- Dense `Vec<T>` and `Vec<FactIdPair>` — no arena, no `HashMap` in the store.
- Reverse index pairs are id-sorted and de-duplicated once at build; each pair
  is two `FactId` unions (small).
- After `build()` the store is shared immutably (e.g. behind `Arc`) across
  threads with zero copy.

## 4. Scale Verification

`half_million_fact_scale_smoke` (500 000 facts):

- Builds through `FactStoreBuilder` in bulk.
- Verifies id-sorted lookups at the head, middle and tail of the id space.
- Verifies a negative lookup (no false positives).
- Validates the full store with zero false positives.
- No timing assertions — the test is deterministic; debug-profile runtime is
  part of the ~16 s fact_store suite.

## 5. Thread-Safety / Sharing

- Every public type is `Send + Sync`; the store contains only owned data.
- `store_is_shared_across_threads` shares one `FactStore` behind `Arc` across
  8 threads performing lookups, queries and validation concurrently — no
  locks, no races.

## 6. Non-Goals

Graph stores, a relationship engine and the Context Compiler are out of scope
for this phase; they are downstream consumers of this store and will carry
their own budgets.