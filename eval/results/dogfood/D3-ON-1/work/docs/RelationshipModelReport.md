# Relationship Model Report

**Phase**: P10.5.0 — Engineering Facts Model
**Status**: IMPLEMENTED

## 1. Model

Relationships are **directional edges** between opaque fact ids:

```
source --kind--> target        (RelationshipFact)
referrer → target              (ReferenceFact)
source → target                (DependencyFact, with version constraint)
```

Every relationship carries its own opaque `RelationshipId`, so the full
edge set is addressable and serialisable. Directionality is first-class:
`source` and `target` are distinct fields and validation treats self-loops
(`source == target`) as an error.

Endpoints are **`FactId` union references** — a relationship may connect any
two facts regardless of kind (symbol→symbol, module→symbol,
package→package, …).

## 2. Relationship Kinds

The complete, closed, language-neutral set of **15 kinds** (including
`Declares`):

| Kind | Direction meaning |
|------|-------------------|
| `Defines` | source defines target |
| `Declares` | source declares target (a container claim, e.g. module declares symbol) |
| `Calls` | source calls target |
| `References` | source references target |
| `Imports` | source imports target |
| `Exports` | source exports target |
| `DependsOn` | source depends on target |
| `Implements` | source implements target |
| `Overrides` | source overrides target |
| `Tests` | source tests target |
| `Builds` | source builds target |
| `Owns` | source owns target |
| `Contains` | source contains target |
| `Friend` | source declares target as friend |
| `Unknown` | producer could not classify |

`RelationshipKind::ALL` (15) is the single source of truth; `parse` rejects
any value outside the set (e.g. `inherits_from` → `None`).

## 3. Dependency Kinds

Dependencies are versioned edges (`source → target` plus an optional
`version_constraint`):

`Direct | Transitive | Optional | Dev | Test | Build | Runtime | Peer | Unknown`

## 4. References

`ReferenceFact` is a specialised, location-bearing edge used for
resolution: `referrer → target` with an optional `SourceLocation` and
metadata. It answers "what references X?" and "where is Y referenced?"
deterministically. The `References` relationship kind covers generic
reference semantics; `ReferenceFact` carries the location.

## 5. Composition Rules

- **Edges reference facts by union id only** — never by name or position,
  so the graph is stable under renames.
- **All endpoints are validated** — every edge endpoint must resolve to a
  known fact (`InvalidReference`).
- **No self loops** — `source == target` on a relationship or reference is
  flagged (`SelfReference`); on a dependency it is flagged as
  `SelfDependency`.
- **Duplicate edge detection** — two relationships sharing the same
  `(kind, source, target)` are flagged as `DuplicateRelationships`.
- **Ownership is expressible as edges** — `Owns`/`Contains`/`Defines`/
  `Declares` are used by the orphan-symbol rule to decide whether a symbol
  is claimed by a module.

## 6. Test Coverage

- `all_relationship_kinds_round_trip` — 15 kinds incl. `Declares`;
  parse/display/`ALL`-list.
- `self_references_are_detected` — relationship + reference self-loops.
- `self_dependencies_are_detected` — dependency self-loop is `SelfDependency`.
- `duplicate_relationships_are_detected` — identical (kind, source, target)
  flagged; different-kind edges are not.
- `invalid_references_are_detected` — unresolved endpoints across
  relationships, API surfaces and references.
- `orphan_symbols_are_detected` + `declares_edge_claims_orphan_symbol` —
  `Owns`/`Declares` edge ownership model.
