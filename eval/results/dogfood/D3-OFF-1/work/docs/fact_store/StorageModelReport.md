# Fact Store Storage Model Report

**Phase**: P10.5.1 — Fact Store Foundation
**Status**: IMPLEMENTED

## 1. Storage Model

The store is **immutable**. Facts are inserted **only during construction**;
the store has no mutation path after build. Storage is dense, sorted `Vec`s —
no `HashMap`, no indirection, no object graph.

```
Language intelligence providers
   │  produce facts (never source)
   ▼
FactStoreBuilder (mutable, absorbing)         FactStore::from_model(&FactsModel)
   │  add_workspace/package/module/symbol/…          │
   │  add_model(&FactsModel)                        │
   └─────────────── build() ◄───────────────────────┘
                        │  FactsModel::builder().build()   (id-sorted model)
                        ▼
                    FactStore  (frozen)
                        │  collection()  → FactCollection
                        │  index()       → FactIndex (primary + reverse)
                        │  lookup()      → FactLookup
                        │  query()       → FactQuery
                        │  validate()    → FactValidationReport
                        │  statistics()  → FactStatistics
                        │  diagnostics() → FactDiagnosticsSummary
                        │  snapshot()    → FactSnapshot
                        ▼
                shared via Arc across threads (read-only)
```

## 2. Supported Operations

| Operation | API | Cost |
|-----------|-----|------|
| Build | `FactStore::build`, `FactStoreBuilder`, `FactStore::from_model` | O(n log n) |
| Lookup | `FactLookup::find` / typed lookups | O(log n), zero alloc |
| Contains | `FactLookup::contains`, `contains_in_kind` | O(log n), zero alloc |
| Count | `len`, `counts`, `FactStatistics` | O(1) |
| Enumerate | `FactCollection::iter`, `FactQuery::enumerate` | allocation-free |
| Snapshot | `FactStore::snapshot` | deterministic canonical bytes |
| Statistics | `FactStore::statistics` | deterministic |
| Diagnostics | `FactStore::diagnostics` | deterministic |

## 3. Ownership Flow

- The store **owns** `FactsModel` (all fact storage) and `FactIndex` (all
  index storage). Both are private; only read views are exposed.
- `FactLookup<'a>` and `FactQuery<'a>` are cheap borrowed views over the
  store, built on demand and dropped without state.

## 4. Immutability Guarantees

- `FactCollection` has no mutation method; `FactStore` fields are private.
- Typed slices returned (`&[WorkspaceFact]`, `&[FactId]`, `&[FactIdPair]`)
  cannot be mutated through the shared reference.
- Verified by `store_has_no_mutation_path_after_build` (repeated
  statistics/diagnostics/snapshot equality) and the concurrency test.

## 5. Serialisation

- `FactStore` is `Clone`, `Debug`, `PartialEq`, `Serialize`/`Deserialize`
  (serde) on every public report and statistics type; `FactCollection` and
  the model satisfy serde too.
- Snapshot bytes are canonical sorted-representation JSON, so identical
  stores produce byte-identical snapshots.