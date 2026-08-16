#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Engineering Facts Model (P10.5.0).
//!
//! The canonical engineering data model consumed by the Engineering
//! Runtime. **Facts are the ONLY public contract between language
//! intelligence providers and the Engineering Runtime** — the runtime never
//! consumes source code directly.
//!
//! # Architecture Contract
//!
//! Facts represent **engineering knowledge**: symbols, modules, packages,
//! workspaces, dependencies, relationships, references, visibility, API
//! surfaces, tests, build targets, diagnostics, architecture rules, source
//! locations and metadata. Facts are **not** syntax, AST, parser output or
//! compiler internals. There is no parser, no AST and no language-specific
//! code anywhere in this module.
//!
//! # Entity Model
//!
//! | Entity | Type | Owned here |
//! |--------|------|-----------|
//! | Workspace | `WorkspaceFact` | `package.rs` |
//! | Package | `PackageFact` | `package.rs` |
//! | Module | `ModuleFact` | `module.rs` |
//! | Symbol | `SymbolFact` | `symbol.rs` |
//! | Test | `TestFact` | `test.rs` |
//! | Build Target | `BuildTargetFact` | `build_target.rs` |
//! | Dependency | `DependencyFact` | `dependency.rs` |
//! | Relationship | `RelationshipFact` | `relationship.rs` |
//! | Reference | `ReferenceFact` | `relationship.rs` |
//! | Diagnostic | `DiagnosticFact` | `diagnostics.rs` |
//! | Architecture Rule | `ArchitectureRuleFact` | `architecture.rs` |
//! | Visibility | `Visibility` | `visibility.rs` |
//! | API Surface | `ApiSurface` | `symbol.rs` |
//! | Source Location | `SourceLocation` | `location.rs` |
//! | Metadata | `FactMetadata` | `metadata.rs` |
//! | Ids | `WorkspaceId` … `ArchitectureRuleId` | `ids.rs` |
//!
//! # Immutability & Performance
//!
//! - Every fact is an immutable value type (public fields, no mutators).
//! - `FactsModel` is immutable after `FactsBuilder::build()`. All storage is
//!   sorted `Vec`s; id lookups use binary search (`O(log n)`) with zero heap
//!   allocation.
//! - No UUID generation, no timestamps, no randomness. IDs are opaque
//!   strings supplied by producers.
//! - Every type is `Send + Sync` and safe to share across threads via `Arc`.

pub mod architecture;
pub mod build_target;
pub mod dependency;
pub mod diagnostics;
pub mod ids;
pub mod location;
pub mod metadata;
pub mod module;
pub mod package;
pub mod relationship;
pub mod symbol;
pub mod test;
pub mod types;
pub mod validation;
pub mod visibility;

#[cfg(test)]
mod tests;

pub use crate::engineering_facts::architecture::ArchitectureRuleFact;
pub use crate::engineering_facts::build_target::{BuildTargetFact, BuildTargetKind};
pub use crate::engineering_facts::dependency::{DependencyFact, DependencyKind};
pub use crate::engineering_facts::diagnostics::DiagnosticFact;
pub use crate::engineering_facts::ids::{
    ArchitectureRuleId, BuildTargetId, DependencyId, DiagnosticId, FactId, FactIdKind, IdKey,
    ModuleId, PackageId, ReferenceId, RelationshipId, SymbolId, TestId, WorkspaceId,
};
pub use crate::engineering_facts::location::{Position, SourceLocation, Span};
pub use crate::engineering_facts::metadata::{Attribute, FactMetadata, Tag};
pub use crate::engineering_facts::module::ModuleFact;
pub use crate::engineering_facts::package::{PackageFact, WorkspaceFact};
pub use crate::engineering_facts::relationship::{
    ReferenceFact, RelationshipFact, RelationshipKind,
};
pub use crate::engineering_facts::symbol::{ApiSurface, SymbolFact, SymbolKind};
pub use crate::engineering_facts::test::TestFact;
pub use crate::engineering_facts::types::{FactKind, Severity};
pub use crate::engineering_facts::validation::{
    FactsValidator, ValidationIssue, ValidationReport, ValidationRule,
};
pub use crate::engineering_facts::visibility::Visibility;

use serde::{Deserialize, Serialize};

use crate::sandbox::RepoState;

/// A typed, borrowed reference to a fact inside a `FactsModel`.
#[derive(Debug)]
pub enum FactRef<'a> {
    Workspace(&'a WorkspaceFact),
    Module(&'a ModuleFact),
    Package(&'a PackageFact),
    Symbol(&'a SymbolFact),
    Test(&'a TestFact),
    BuildTarget(&'a BuildTargetFact),
    Dependency(&'a DependencyFact),
    Relationship(&'a RelationshipFact),
    Reference(&'a ReferenceFact),
    Diagnostic(&'a DiagnosticFact),
    ArchitectureRule(&'a ArchitectureRuleFact),
}

/// Per-category fact counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModelCounts {
    pub workspaces: usize,
    pub modules: usize,
    pub packages: usize,
    pub symbols: usize,
    pub tests: usize,
    pub build_targets: usize,
    pub dependencies: usize,
    pub relationships: usize,
    pub references: usize,
    pub diagnostics: usize,
    pub architecture_rules: usize,
    pub total: usize,
}

/// An immutable, frozen collection of engineering facts.
///
/// Build once via [`FactsBuilder`]; afterwards the model has no mutation
/// path. Every storage vector is sorted by opaque id, enabling `O(log n)`
/// binary-search lookups that allocate nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactsModel {
    workspaces: Vec<WorkspaceFact>,
    modules: Vec<ModuleFact>,
    packages: Vec<PackageFact>,
    symbols: Vec<SymbolFact>,
    tests: Vec<TestFact>,
    build_targets: Vec<BuildTargetFact>,
    dependencies: Vec<DependencyFact>,
    relationships: Vec<RelationshipFact>,
    references: Vec<ReferenceFact>,
    diagnostics: Vec<DiagnosticFact>,
    architecture_rules: Vec<ArchitectureRuleFact>,
    /// Repository state at the time facts were generated. Used for
    /// freshness comparison against the current repository state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generation_repo_state: Option<RepoState>,
}

impl Default for FactsModel {
    fn default() -> Self {
        FactsModel::empty()
    }
}

impl FactsModel {
    /// An empty, immutable model.
    pub fn empty() -> Self {
        FactsModel {
            workspaces: Vec::new(),
            modules: Vec::new(),
            packages: Vec::new(),
            symbols: Vec::new(),
            tests: Vec::new(),
            build_targets: Vec::new(),
            dependencies: Vec::new(),
            relationships: Vec::new(),
            references: Vec::new(),
            diagnostics: Vec::new(),
            architecture_rules: Vec::new(),
            generation_repo_state: None,
        }
    }

    /// Start a mutable builder that freezes into an immutable model.
    pub fn builder() -> FactsBuilder {
        FactsBuilder::new()
    }

    // ── Sliced views (immutable, allocation-free) ─────────────────────────

    pub fn workspaces(&self) -> &[WorkspaceFact] {
        &self.workspaces
    }

    pub fn modules(&self) -> &[ModuleFact] {
        &self.modules
    }

    pub fn packages(&self) -> &[PackageFact] {
        &self.packages
    }

    pub fn symbols(&self) -> &[SymbolFact] {
        &self.symbols
    }

    pub fn tests(&self) -> &[TestFact] {
        &self.tests
    }

    pub fn build_targets(&self) -> &[BuildTargetFact] {
        &self.build_targets
    }

    pub fn dependencies(&self) -> &[DependencyFact] {
        &self.dependencies
    }

    pub fn relationships(&self) -> &[RelationshipFact] {
        &self.relationships
    }

    pub fn references(&self) -> &[ReferenceFact] {
        &self.references
    }

    pub fn diagnostics(&self) -> &[DiagnosticFact] {
        &self.diagnostics
    }

    pub fn architecture_rules(&self) -> &[ArchitectureRuleFact] {
        &self.architecture_rules
    }

    // ── O(log n) binary-search lookups; no allocation ─────────────────────

    /// True when any fact in the model carries `id`.
    pub fn contains(&self, id: &FactId) -> bool {
        self.find(id).is_some()
    }

    /// Locate any fact by union id.
    pub fn find(&self, id: &FactId) -> Option<FactRef<'_>> {
        match id {
            FactId::Workspace(v) => self.workspace(v).map(FactRef::Workspace),
            FactId::Module(v) => self.module(v).map(FactRef::Module),
            FactId::Package(v) => self.package(v).map(FactRef::Package),
            FactId::Symbol(v) => self.symbol(v).map(FactRef::Symbol),
            FactId::Test(v) => self.test(v).map(FactRef::Test),
            FactId::BuildTarget(v) => self.build_target(v).map(FactRef::BuildTarget),
            FactId::Dependency(v) => self.dependency(v).map(FactRef::Dependency),
            FactId::Relationship(v) => self.relationship(v).map(FactRef::Relationship),
            FactId::Reference(v) => self.reference(v).map(FactRef::Reference),
            FactId::Diagnostic(v) => self.diagnostic(v).map(FactRef::Diagnostic),
            FactId::ArchitectureRule(v) => self.architecture_rule(v).map(FactRef::ArchitectureRule),
        }
    }

    /// Look up a workspace by opaque id.
    pub fn workspace(&self, id: &WorkspaceId) -> Option<&WorkspaceFact> {
        self.workspaces
            .binary_search_by(|f| f.id.cmp(id))
            .ok()
            .map(|i| &self.workspaces[i])
    }

    /// Look up a module by opaque id.
    pub fn module(&self, id: &ModuleId) -> Option<&ModuleFact> {
        self.modules
            .binary_search_by(|f| f.id.cmp(id))
            .ok()
            .map(|i| &self.modules[i])
    }

    /// Look up a package by opaque id.
    pub fn package(&self, id: &PackageId) -> Option<&PackageFact> {
        self.packages
            .binary_search_by(|f| f.id.cmp(id))
            .ok()
            .map(|i| &self.packages[i])
    }

    /// Look up a symbol by opaque id.
    pub fn symbol(&self, id: &SymbolId) -> Option<&SymbolFact> {
        self.symbols
            .binary_search_by(|f| f.id.cmp(id))
            .ok()
            .map(|i| &self.symbols[i])
    }

    /// Look up a test by opaque id.
    pub fn test(&self, id: &TestId) -> Option<&TestFact> {
        self.tests
            .binary_search_by(|f| f.id.cmp(id))
            .ok()
            .map(|i| &self.tests[i])
    }

    /// Look up a build target by opaque id.
    pub fn build_target(&self, id: &BuildTargetId) -> Option<&BuildTargetFact> {
        self.build_targets
            .binary_search_by(|f| f.id.cmp(id))
            .ok()
            .map(|i| &self.build_targets[i])
    }

    /// Look up a dependency by opaque id.
    pub fn dependency(&self, id: &DependencyId) -> Option<&DependencyFact> {
        self.dependencies
            .binary_search_by(|f| f.id.cmp(id))
            .ok()
            .map(|i| &self.dependencies[i])
    }

    /// Look up a relationship by opaque id.
    pub fn relationship(&self, id: &RelationshipId) -> Option<&RelationshipFact> {
        self.relationships
            .binary_search_by(|f| f.id.cmp(id))
            .ok()
            .map(|i| &self.relationships[i])
    }

    /// Look up a reference by opaque id.
    pub fn reference(&self, id: &ReferenceId) -> Option<&ReferenceFact> {
        self.references
            .binary_search_by(|f| f.id.cmp(id))
            .ok()
            .map(|i| &self.references[i])
    }

    /// Look up a diagnostic by opaque id.
    pub fn diagnostic(&self, id: &DiagnosticId) -> Option<&DiagnosticFact> {
        self.diagnostics
            .binary_search_by(|f| f.id.cmp(id))
            .ok()
            .map(|i| &self.diagnostics[i])
    }

    /// Look up an architecture rule by opaque id.
    pub fn architecture_rule(&self, id: &ArchitectureRuleId) -> Option<&ArchitectureRuleFact> {
        self.architecture_rules
            .binary_search_by(|f| f.id.cmp(id))
            .ok()
            .map(|i| &self.architecture_rules[i])
    }

    // ── Aggregation ───────────────────────────────────────────────────────

    /// Total number of facts across all categories.
    pub fn len(&self) -> usize {
        self.workspaces.len()
            + self.modules.len()
            + self.packages.len()
            + self.symbols.len()
            + self.tests.len()
            + self.build_targets.len()
            + self.dependencies.len()
            + self.relationships.len()
            + self.references.len()
            + self.diagnostics.len()
            + self.architecture_rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Per-category counts.
    pub fn counts(&self) -> ModelCounts {
        ModelCounts {
            workspaces: self.workspaces.len(),
            modules: self.modules.len(),
            packages: self.packages.len(),
            symbols: self.symbols.len(),
            tests: self.tests.len(),
            build_targets: self.build_targets.len(),
            dependencies: self.dependencies.len(),
            relationships: self.relationships.len(),
            references: self.references.len(),
            diagnostics: self.diagnostics.len(),
            architecture_rules: self.architecture_rules.len(),
            total: self.len(),
        }
    }

    /// Run the deterministic validation rules over this model.
    pub fn validate(&self) -> ValidationReport {
        FactsValidator::validate(self)
    }

    /// Generation-time repository state, if captured during init.
    pub fn generation_repo_state(&self) -> Option<&RepoState> {
        self.generation_repo_state.as_ref()
    }

    /// Set the generation-time repository state.
    pub fn with_generation_repo_state(mut self, state: RepoState) -> Self {
        self.generation_repo_state = Some(state);
        self
    }
}

/// A mutable, absorbing builder that freezes into an immutable `FactsModel`.
///
/// Facts may be added in any order; `build()` sorts every category by
/// opaque id, so the frozen model (and its serialised form) is fully
/// deterministic.
#[derive(Debug, Default)]
pub struct FactsBuilder {
    workspaces: Vec<WorkspaceFact>,
    modules: Vec<ModuleFact>,
    packages: Vec<PackageFact>,
    symbols: Vec<SymbolFact>,
    tests: Vec<TestFact>,
    build_targets: Vec<BuildTargetFact>,
    dependencies: Vec<DependencyFact>,
    relationships: Vec<RelationshipFact>,
    references: Vec<ReferenceFact>,
    diagnostics: Vec<DiagnosticFact>,
    architecture_rules: Vec<ArchitectureRuleFact>,
    generation_repo_state: Option<RepoState>,
}

impl FactsBuilder {
    pub fn new() -> Self {
        FactsBuilder::default()
    }

    pub fn add_workspace(&mut self, fact: WorkspaceFact) -> &mut Self {
        self.workspaces.push(fact);
        self
    }

    pub fn add_module(&mut self, fact: ModuleFact) -> &mut Self {
        self.modules.push(fact);
        self
    }

    pub fn add_package(&mut self, fact: PackageFact) -> &mut Self {
        self.packages.push(fact);
        self
    }

    pub fn add_symbol(&mut self, fact: SymbolFact) -> &mut Self {
        self.symbols.push(fact);
        self
    }

    pub fn add_test(&mut self, fact: TestFact) -> &mut Self {
        self.tests.push(fact);
        self
    }

    pub fn add_build_target(&mut self, fact: BuildTargetFact) -> &mut Self {
        self.build_targets.push(fact);
        self
    }

    pub fn add_dependency(&mut self, fact: DependencyFact) -> &mut Self {
        self.dependencies.push(fact);
        self
    }

    pub fn add_relationship(&mut self, fact: RelationshipFact) -> &mut Self {
        self.relationships.push(fact);
        self
    }

    pub fn add_reference(&mut self, fact: ReferenceFact) -> &mut Self {
        self.references.push(fact);
        self
    }

    pub fn add_diagnostic(&mut self, fact: DiagnosticFact) -> &mut Self {
        self.diagnostics.push(fact);
        self
    }

    pub fn add_architecture_rule(&mut self, fact: ArchitectureRuleFact) -> &mut Self {
        self.architecture_rules.push(fact);
        self
    }

    /// Set the generation-time repository state.
    pub fn with_generation_repo_state(mut self, state: RepoState) -> Self {
        self.generation_repo_state = Some(state);
        self
    }

    /// Freeze into an immutable, id-sorted `FactsModel`.
    pub fn build(self) -> FactsModel {
        fn sort_by_id<T>(v: &mut Vec<T>)
        where
            T: IdCarrier,
        {
            v.sort_by(|a, b| a.fact_id().cmp(b.fact_id()));
        }

        let mut model = FactsModel {
            workspaces: self.workspaces,
            modules: self.modules,
            packages: self.packages,
            symbols: self.symbols,
            tests: self.tests,
            build_targets: self.build_targets,
            dependencies: self.dependencies,
            relationships: self.relationships,
            references: self.references,
            diagnostics: self.diagnostics,
            architecture_rules: self.architecture_rules,
            generation_repo_state: self.generation_repo_state,
        };
        sort_by_id(&mut model.workspaces);
        sort_by_id(&mut model.modules);
        sort_by_id(&mut model.packages);
        sort_by_id(&mut model.symbols);
        sort_by_id(&mut model.tests);
        sort_by_id(&mut model.build_targets);
        sort_by_id(&mut model.dependencies);
        sort_by_id(&mut model.relationships);
        sort_by_id(&mut model.references);
        sort_by_id(&mut model.diagnostics);
        sort_by_id(&mut model.architecture_rules);
        model
    }
}

/// Internal helper trait so the builder can sort every category uniformly.
/// The opaque id payload is returned as `&str`, keeping the sort
/// allocation-free and identical to the binary-search comparison order.
trait IdCarrier {
    fn fact_id(&self) -> &str;
}

macro_rules! id_carrier {
    ($($ty:ty),* $(,)?) => {
        $(
            impl IdCarrier for $ty {
                fn fact_id(&self) -> &str {
                    self.id.as_str()
                }
            }
        )*
    };
}

id_carrier! {
    WorkspaceFact,
    ModuleFact,
    PackageFact,
    SymbolFact,
    TestFact,
    BuildTargetFact,
    DependencyFact,
    RelationshipFact,
    ReferenceFact,
    DiagnosticFact,
    ArchitectureRuleFact,
}
