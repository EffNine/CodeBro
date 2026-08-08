# Performance Budget Report

**Phase**: P10.5.0 — Engineering Facts Model
**Status**: IMPLEMENTED

## 1. Budget Statement

| Requirement | Design | Verified |
|-------------|--------|----------|
| Zero heap allocations during simple lookups | id lookups are binary searches over sorted `Vec` slices returning `&T`; `FactMetadata::get`/`has_tag` are partition-point searches over sorted vectors | by construction (no `String`, `Vec`, or container allocated in lookup paths) |
| Support millions of facts | flat `Vec` storage per category; O(n) build, O(log n) lookup; no per-fact indirection or object graph | `million_fact_scale_smoke` (250 000 symbols) runs in the ~0.4 s suite |
| Immutable after creation | `FactsModel` frozen by `FactsBuilder::build()`; facts are value types with no mutators | compile-time; determinism tests |

## 2. Lookup Path

```
model.symbol(&SymbolId)      →  binary search over symbols (sorted by id)
model.workspace(&WorkspaceId)→  binary search over workspaces
model.contains(&FactId)      →  per-category binary searches via find()
model.find(&FactId)          →  dispatch on kind → one binary search
metadata.get(&str)           →  partition_point over sorted attributes
metadata.has_tag(&str)       →  binary search over sorted tags
```

All of the above return borrowed data and perform **zero heap allocations**.
Iteration over any category is a plain slice walk. Typed IDs are compared
by their opaque string payload (`Ord` over the newtype), so lookups stay
`O(log n)` with no conversion.

## 3. Memory Model

- Storage is dense `Vec<T>` (no `HashMap`/`Rc`/indirection in the model
  itself). `T` sizes are dominated by the producer's strings.
- The builder is the only place vectors grow; after `build()` the model
  is shared immutably (e.g. behind `Arc`) across threads with zero copy.
- `FactMetadata` canonicalises tags/attributes once at build time
  (sort + dedup), keeping per-fact metadata small and comparable.

## 4. Scale Verification

`million_fact_scale_smoke`:

- Builds a 250 000-symbol model in insertion order.
- Verifies id-sorted lookup at the head, middle and tail of the id space.
- Verifies a negative lookup (no false positives).
- Runs `validate()` over the full model with zero orphan/duplicate
  findings.

Run time in debug profile: part of the engineering_facts suite (~0.4 s
across all 36 tests). No timing assertions are used, keeping the test
deterministic.

## 5. Thread-Safety / Sharing

- Every public type is `Send + Sync`; facts contain only owned data.
- `model_is_send_and_sync_across_threads` shares one `FactsModel` behind
  `Arc` across 8 threads performing union/typed lookups and validation
  concurrently — no locks, no races (read-only).

## 6. Non-Goals

Graph stores, indexing and the Engineering Runtime are out of scope for
this phase; they are downstream consumers of this fact model and own their
own budgets (see `docs/engineering_runtime/PerformanceBudget.md`).