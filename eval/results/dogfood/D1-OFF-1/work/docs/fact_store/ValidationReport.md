# Fact Store Validation Report

**Phase**: P10.5.1 — Fact Store Foundation
**Status**: IMPLEMENTED

> This is the store-level validation report (P10.5.1). The P10.5.0
> model-level report remains at `docs/ValidationReport.md`.

## 1. Rule Set

`FactValidation` applies **five deterministic rules** over a frozen
`FactStore` and emits a sorted `FactValidationReport`.

| Rule | Severity | Checks |
|------|----------|--------|
| `DuplicateFacts` | Error | an opaque id occurring more than once anywhere in the collection |
| `BrokenIndex` | Error | a primary or reverse index entry referencing an id that does not resolve in the collection |
| `MissingIds` | Error | a collection record absent from the primary index of its kind (index incompleteness) |
| `OrphanRecords` | Warning | a collection record scoped by no reverse index (workspaces and dependencies exempt by construction) |
| `SchemaMismatch` | Error | a primary index entry whose id kind does not match the index kind |

## 2. Semantics

- `FactValidationReport { issues, checked_entities, checked_index_entries }`.
- `passed()` = zero `Error`/`Fatal` findings; orphan records are advisory.
- `count_by_rule`, `error_count`, `warning_count` for reporting.
- Issues are sorted by `(rule, entity, message)` before emission, so a given
  store always produces a byte-identical report.

## 3. Determinism

- Collection and index storage are id-sorted at build time.
- Ids are collected in a fixed category order; reverse coverage uses a set
  (membership only, never iterated for output).
- Verified by `validation_is_deterministic_and_sorted` (report equality +
  sorted issue invariant).

## 4. Test Results (validation-focused)

| Test | Asserts |
|------|---------|
| `clean_store_passes_validation` | zero issues on the 15-fact sample store |
| `duplicate_facts_are_detected` | two facts sharing an opaque id flagged |
| `broken_index_is_detected` | dangling reverse entry flagged |
| `missing_ids_are_detected` | record absent from primary index flagged |
| `schema_mismatch_is_detected` | wrong-kind primary entry flagged |
| `orphan_records_are_detected_as_warnings` | scopeless record warned; still `passed()` |
| `validation_is_deterministic_and_sorted` | equality + ordering invariants |
| `rules_parse_and_round_trip` | 5 rules parse/display/`ALL` |
| `half_million_fact_scale_smoke` | scale model validates cleanly, no false positives |

## 5. Scale

`half_million_fact_scale_smoke` validates a 500 000-fact store
(1 workspace + 1 package + 250 000 modules + 250 000 symbols): zero
duplicate, missing-id and orphan findings — O(n) validation with no false
positives at scale.