# Entity Schema Report

**Phase**: P10.5.0 — Engineering Facts Model
**Status**: IMPLEMENTED

## 1. Entity Requirements

Every entity below is **immutable**, `Clone`, `Debug`, `Eq`, `Hash` (where
it participates in set/key semantics), `Serialize`, `Deserialize`,
`Send + Sync`. All IDs are **opaque, strongly-typed** per entity. There is
**no UUID generation, no timestamp and no randomness** anywhere.

Detailed schema in the code itself; this report lists the entity API.

## 2. Identity — `ids.rs`

### Strongly-typed IDs
`WorkspaceId`, `PackageId`, `ModuleId`, `SymbolId`, `TestId`,
`BuildTargetId`, `DependencyId`, `RelationshipId`, `ReferenceId`,
`DiagnosticId`, `ArchitectureRuleId` — opaque newtypes over a
producer-supplied string, `serde(transparent)`, `Ord`, `Hash`, `Display`,
`AsRef<str>`, `From<&str>`, `From<String>`.

### `FactId` (union reference)
`Workspace | Package | Module | Symbol | Test | BuildTarget | Dependency |
Relationship | Reference | Diagnostic | ArchitectureRule`
- `FactId::new(kind, value)`, `kind() -> FactKind`, `as_str() -> &str`.
- `From<EachTypedId>` and `From<&EachTypedId>` for lossless conversion.
- Used for cross-entity endpoints: relationship/reference/dependency
  source+target, test `target`, diagnostic `related`, rule `from`/`to`.

### `FactKind`
`Workspace | Module | Package | Symbol | Test | BuildTarget | Dependency |
Relationship | Reference | Diagnostic | ArchitectureRule` with `ALL`,
`as_str`, `parse` (unknown → `None`).

### `Severity`
`Info | Warning | Error | Fatal` with `ALL`, `as_str`, `parse`.

## 3. Symbols — `symbol.rs`

### `SymbolKind`
`Function | Method | Class | Struct | Enum | Trait | Interface | TypeAlias
| Variable | Constant | Field | Parameter | Macro | Constructor | Operator
| Namespace | Import | Export | Unknown`

### `ApiSurface`
- `exports: Vec<SymbolId>`
- `entry_points: Vec<SymbolId>`

### `SymbolFact`
- `id: SymbolId`
- `name: String`
- `kind: SymbolKind`
- `visibility: Visibility`
- `location: SourceLocation`
- `module: Option<ModuleId>`
- `signature: Option<String>` — engineering signature, not source syntax
- `metadata: FactMetadata`

## 4. Containers

### `ModuleFact` — `module.rs`
`id: ModuleId`, `name`, `package: Option<PackageId>`, `path: Option<String>`,
`visibility`, `location`, `api: ApiSurface`, `metadata`.

### `PackageFact` — `package.rs`
`id: PackageId`, `name`, `version: Option<String>`,
`workspace: Option<WorkspaceId>`, `language: Option<String>`,
`build_targets: Vec<BuildTargetId>`, `metadata`.

### `WorkspaceFact` — `package.rs`
`id: WorkspaceId`, `name`, `root: Option<String>`,
`packages: Vec<PackageId>`, `metadata`.

## 5. Edges — `dependency.rs` / `relationship.rs`

### `DependencyKind`
`Direct | Transitive | Optional | Dev | Test | Build | Runtime | Peer |
Unknown`

### `DependencyFact` (directed `source → target`)
`id: DependencyId`, `kind`, `source: FactId`, `target: FactId`,
`version_constraint: Option<String>`, `metadata`.

### `RelationshipKind` (15 kinds, incl. `Declares`)
`Defines | Declares | Calls | References | Imports | Exports | DependsOn |
Implements | Overrides | Tests | Builds | Owns | Contains | Friend |
Unknown`

### `RelationshipFact` (directed `source --kind--> target`)
`id: RelationshipId`, `kind`, `source: FactId`, `target: FactId`,
`location: Option<SourceLocation>`, `metadata`.

### `ReferenceFact` (directed `referrer → target`)
`id: ReferenceId`, `referrer: FactId`, `target: FactId`,
`location: Option<SourceLocation>`, `metadata`.

## 6. Test / Build / Architecture

### `TestFact` — `test.rs`
`id: TestId`, `name`, `target: Option<FactId>`, `tested: Vec<SymbolId>`,
`location: Option<SourceLocation>`, `metadata`.

### `BuildTargetFact` / `BuildTargetKind` — `build_target.rs`
Binary | Library | Test | Example | Bench | Unknown. Fields: `id:
BuildTargetId`, `name`, `kind`, `language: Option<String>`,
`package: Option<PackageId>`, `metadata`.

### `ArchitectureRuleFact` — `architecture.rs`
`id: ArchitectureRuleId`, `name`, `from: Option<FactId>`,
`to: Option<FactId>`, `description: Option<String>`, `metadata`.

## 7. Cross-cutting

### `DiagnosticFact` — `diagnostics.rs`
`id: DiagnosticId`, `severity: Severity`, `message`,
`code: Option<String>`, `location: Option<SourceLocation>`,
`related: Vec<FactId>` (union refs), `metadata`.

### `Visibility` — `visibility.rs`
`Public | Protected | Internal | Private | Package | Unknown` with
`parse` (rejects any language-specific value), `is_resolved`.

### `SourceLocation` — `location.rs`
- `workspace: Option<WorkspaceId>`
- `package: Option<PackageId>`
- `module: Option<ModuleId>`
- `file: Option<String>` — canonical path
- `line: Option<u32>`, `column: Option<u32>` — direct point
- `span: Option<Span>` — `Span { start, end: Position }`,
  `Position { line, column }` (1-based integers; no parser)

### `FactMetadata` — `metadata.rs`
`tags: Vec<Tag>` (sorted, de-duplicated), `attributes: Vec<Attribute>`
(sorted by key/value), `description: Option<String>`,
`language: Option<String>`; `has_tag`/`get` are allocation-free binary
searches; `FactMetadataBuilder` canonicalises on `build`.

## 8. Aggregate — `mod.rs`

### `FactsModel`
Immutable, id-sorted vectors per category. API:
- sliced views: `workspaces()`, `modules()`, `packages()`, `symbols()`,
  `tests()`, `build_targets()`, `dependencies()`, `relationships()`,
  `references()`, `diagnostics()`, `architecture_rules()`
- `O(log n)` lookups: `contains(FactId)`, `find → FactRef`, and one getter
  per category (`workspace(id)`, `module(id)`, `symbol(id)`, …)
- `len()`, `is_empty()`, `counts() → ModelCounts`, `validate() →
  ValidationReport`

### `FactsBuilder`
One `add_*` per category; `build()` sorts every category by opaque id.

### `FactRef`
Typed borrowed enum over all eleven fact kinds, returned by `find`.

## 8. Schema Validation

- Strongly-typed closed enums guarantee language-neutral values.
- `Visibility::parse`/`RelationshipKind::parse`/… return `None` for
  out-of-model strings (covered by `all_*_round_trip` tests).
- Full aggregate round-trips through JSON and TOML byte-identically
  (`full_model_serde_round_trip`, `serde_round_trip_is_byte_identical`).