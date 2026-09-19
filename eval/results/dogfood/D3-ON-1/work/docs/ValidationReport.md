# Validation Report

**Phase**: P10.5.0 — Engineering Facts Model
**Status**: IMPLEMENTED

## 1. Rule Set

`FactsValidator` applies **eight deterministic rules** over a frozen
`FactsModel`. Results are emitted as an immutable, sorted
`ValidationReport`.

| Rule | Severity | Checks |
|------|----------|--------|
| `DuplicateIds` | Error | any opaque id appearing more than once anywhere in the model (across all categories) |
| `DuplicateRelationships` | Error | two relationships sharing the same `(kind, source, target)` |
| `InvalidReference` | Error | relationship/reference/dependency/API/ownership/test endpoint that does not resolve to a known fact |
| `SelfReference` | Error | relationship or reference where `source == target` |
| `SelfDependency` | Error | dependency where `source == target` |
| `BrokenLocation` | Error | a `SourceLocation` pointing at an unknown workspace, package or module id |
| `InvalidVisibility` | Warning | symbol or module whose visibility is `Unknown` (unresolved) |
| `OrphanSymbol` | Warning | symbol with no owning module and no `Owns`/`Contains`/`Defines`/`Declares` edge from a module |

## 2. Semantics

- `ValidationReport { issues, checked_entities }`.
- `passed()` = zero `Error`/`Fatal` findings (warnings permitted —
  unresolved visibility and orphans are advisory).
- `count_by_rule`, `error_count`, `warning_count` for reporting.
- Issues are sorted by `(rule, entity, message)` before emission, so a
  given model always produces a byte-identical report.

## 3. Coverage Per Endpoint

Every reference field is validated against the id universe:

- `RelationshipFact.{source,target}`
- `ReferenceFact.{referrer,target}`
- `DependencyFact.{source,target}`
- `SymbolFact.module`, `ModuleFact.package`,
  `ModuleFact.api.{exports,entry_points}`
- `PackageFact.{workspace,build_targets}`, `WorkspaceFact.packages`
- `TestFact.{target,tested}`, `BuildTargetFact.package`
- `DiagnosticFact.related`, `ArchitectureRuleFact.{from,to}`
- `SourceLocation.{workspace,package,module}` → `BrokenLocation`

## 4. Determinism

- Storage is id-sorted at build time.
- The id universe is collected in a fixed category order.
- Issues are fully sorted before emission.
- Verified by `validation_is_deterministic` (report equality + identical
  serialisation) and `validation_issues_are_sorted`.

## 5. Test Results (validation-focused)

```
cargo test --bin codebro engineering_facts
36 passed; 0 failed; 0 ignored
```

| Test | Asserts |
|------|---------|
| `clean_model_passes_validation` | zero issues on a well-formed model |
| `duplicate_ids_are_detected` | intra-category duplicate flagged |
| `duplicate_ids_across_categories_are_detected` | cross-category duplicate flagged |
| `duplicate_relationships_are_detected` | same (kind, source, target) flagged |
| `invalid_references_are_detected` | 2 unresolved endpoints flagged |
| `self_references_are_detected` | relationship + reference self-loops |
| `self_dependencies_are_detected` | dependency self-loop as `SelfDependency` |
| `broken_locations_are_detected` | 2 unresolved location scope ids |
| `orphan_symbols_are_detected` | only the unclaimed symbol flagged |
| `declares_edge_claims_orphan_symbol` | `Declares` edge prevents orphan |
| `unresolved_visibility_is_warned` | 2 warnings; still `passed()` |
| `validation_is_deterministic` | equal inputs → equal reports |
| `validation_issues_are_sorted` | issue ordering invariant |
| `validation_rules_parse_and_round_trip` | 8 rules parse/display/`ALL` |

## 6. Scale

`million_fact_scale_smoke` validates a 250 000-symbol model: zero orphan or
duplicate findings, confirming validation is O(n) with no false positives
at scale.