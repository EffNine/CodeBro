# Fact Store Index Report

**Phase**: P10.5.1 — Fact Store Foundation
**Status**: IMPLEMENTED

## 1. Index Set

`FactIndex` is built once during construction and is **read-only**. It
carries one primary index per entity id plus four reverse scope indexes.

| Index | Owner | Members | Build source |
|-------|-------|---------|--------------|
| `facts_of_kind(Workspace)` | — | every workspace id | `WorkspaceFact.id` |
| `facts_of_kind(Package)` | — | every package id | `PackageFact.id` |
| `facts_of_kind(Module)` | — | every module id | `ModuleFact.id` |
| `facts_of_kind(Symbol)` | — | every symbol id | `SymbolFact.id` |
| `facts_of_kind(Test)` | — | every test id | `TestFact.id` |
| `facts_of_kind(BuildTarget)` | — | every build-target id | `BuildTargetFact.id` |
| `facts_of_kind(Dependency)` | — | every dependency id | `DependencyFact.id` |
| `facts_of_kind(Relationship)` | — | every relationship id | `RelationshipFact.id` |
| `facts_of_kind(Reference)` | — | every reference id | `ReferenceFact.id` |
| `facts_of_kind(Diagnostic)` | — | every diagnostic id | `DiagnosticFact.id` |
| `facts_of_kind(ArchitectureRule)` | — | every architecture-rule id | `ArchitectureRuleFact.id` |
| `facts_in_workspace` | `WorkspaceId` | packages + located facts | workspace-declared packages, `package.workspace`, `SourceLocation.workspace` |
| `facts_in_package` | `PackageId` | modules, build targets + located facts | `module.package`, `build_target.package`, `SourceLocation.package` |
| `facts_in_module` | `ModuleId` | symbols + located facts | `symbol.module`, `SourceLocation.module` |
| `facts_in_symbol` | `SymbolId` | tests, refs, rels, diagnostics, exporting modules, rules | `test.tested`, `reference.{referrer,target}`, `relationship.{source,target}`, `diagnostic.related`, `module.api.{exports,entry_points}`, `rule.{from,to}` |

## 2. Semantics

- **Primary indexes** hold the complete, sorted `FactId` set of every entity
  kind, mirroring the frozen model. Membership and range checks are `O(log n)`.
- **Reverse indexes** are `Vec<FactIdPair (owner, member)>`, sorted by
  `(owner, member)` and de-duplicated. All reverse entries are **pure
  projections of declared field references** — never transitive traversal.
- Indexes are deterministic: identical stores produce byte-identical indexes
  (sorted storage, no HashMap iteration in output).

## 3. Read-only Guarantee

- `FactIndex` and `ReverseIndex` field lists are private; only read accessors
  exist. There are no mutators in the public API.
- `ReverseIndex::get` returns a contiguous `&[FactIdPair]` slice (partition
  point + linear equal-range scan), allocating nothing.

## 4. Index Lookups

```
index.facts_of_kind(kind)         — &[FactId]          O(log n)
index.contains_in_kind(kind, id)  → bool               O(log n)
index.facts_in_workspace(id)      → &[FactIdPair]      O(log n + k)
index.facts_in_package(id)        → &[FactIdPair]      O(log n + k)
index.facts_in_module(id)         → &[FactIdPair]      O(log n + k)
index.facts_in_symbol(id)         → &[FactIdPair]      O(log n + k)
index.primary_len() / reverse_len → usize              O(1)
```

## 5. Coverage on the Sample Store (15 facts)

| Reverse index | Entries |
|---------------|---------|
| by_workspace | 10 |
| by_package | 9 |
| by_module | 6 |
| by_symbol | 9 |
| total | 34 |

Primary index total: 15. All tested in `statistics_are_correct_and_deterministic`
and the `reverse_index_*_projection` tests.

## 6. Test Results (index-focused)

```
cargo test --bin codebro fact_store
39 passed; 0 failed; 0 ignored; finished in ~16 s (debug)
```

`primary_index_covers_every_fact`, `primary_index_is_sorted`,
`reverse_index_*_projection`, `reverse_indexes_are_sorted_and_deduped`,
`index_is_deterministic`.