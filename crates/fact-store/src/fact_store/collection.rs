#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! The immutable fact repository (P10.5.1).
//!
//! [`FactCollection`] is the canonical repository for engineering facts. It
//! wraps a frozen [`FactsModel`] (P10.5.0) and exposes typed slices, id
//! lookups, membership checks and an allocation-free enumerator. There is no
//! mutation path: a collection is built once and shared immutably.

use crate::engineering_facts::{
    ArchitectureRuleFact, BuildTargetFact, DependencyFact, DiagnosticFact, FactId, FactRef,
    FactsModel, ModelCounts, ModuleFact, ModuleId, PackageFact, PackageId, ReferenceFact,
    RelationshipFact, SymbolFact, SymbolId, TestFact, WorkspaceFact, WorkspaceId,
};
use serde::{Deserialize, Serialize};

/// An immutable, frozen repository of engineering facts.
///
/// Built from a `FactsModel`; afterwards there is no way to add, remove or
/// alter a fact. `Clone`, `PartialEq`, `Send + Sync`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactCollection {
    model: FactsModel,
}

impl FactCollection {
    /// Freeze a `FactsModel` into an immutable collection.
    pub fn from_model(model: FactsModel) -> Self {
        FactCollection { model }
    }

    /// Borrow the underlying frozen model.
    pub fn model(&self) -> &FactsModel {
        &self.model
    }

    /// The category slice of every fact kind. Each slice is id-sorted.
    pub fn workspaces(&self) -> &[WorkspaceFact] {
        self.model.workspaces()
    }
    pub fn modules(&self) -> &[ModuleFact] {
        self.model.modules()
    }
    pub fn packages(&self) -> &[PackageFact] {
        self.model.packages()
    }
    pub fn symbols(&self) -> &[SymbolFact] {
        self.model.symbols()
    }
    pub fn tests(&self) -> &[TestFact] {
        self.model.tests()
    }
    pub fn build_targets(&self) -> &[BuildTargetFact] {
        self.model.build_targets()
    }
    pub fn dependencies(&self) -> &[DependencyFact] {
        self.model.dependencies()
    }
    pub fn relationships(&self) -> &[RelationshipFact] {
        self.model.relationships()
    }
    pub fn references(&self) -> &[ReferenceFact] {
        self.model.references()
    }
    pub fn diagnostics(&self) -> &[DiagnosticFact] {
        self.model.diagnostics()
    }
    pub fn architecture_rules(&self) -> &[ArchitectureRuleFact] {
        self.model.architecture_rules()
    }

    /// Total number of facts across all categories.
    pub fn len(&self) -> usize {
        self.model.len()
    }

    pub fn is_empty(&self) -> bool {
        self.model.is_empty()
    }

    /// Per-category counts.
    pub fn counts(&self) -> ModelCounts {
        self.model.counts()
    }

    /// True when any fact in the collection carries `id`.
    pub fn contains(&self, id: &FactId) -> bool {
        self.model.contains(id)
    }

    /// Locate any fact by union id.
    pub fn find(&self, id: &FactId) -> Option<FactRef<'_>> {
        self.model.find(id)
    }

    /// Locate a workspace by opaque id.
    pub fn workspace(&self, id: &WorkspaceId) -> Option<&WorkspaceFact> {
        self.model.workspace(id)
    }

    /// Locate a module by opaque id.
    pub fn module(&self, id: &ModuleId) -> Option<&ModuleFact> {
        self.model.module(id)
    }

    /// Locate a package by opaque id.
    pub fn package(&self, id: &PackageId) -> Option<&PackageFact> {
        self.model.package(id)
    }

    /// Locate a symbol by opaque id.
    pub fn symbol(&self, id: &SymbolId) -> Option<&SymbolFact> {
        self.model.symbol(id)
    }

    // Remaining typed lookups delegate to the frozen model.
    pub fn test(&self, id: &crate::engineering_facts::TestId) -> Option<&TestFact> {
        self.model.test(id)
    }
    pub fn build_target(
        &self,
        id: &crate::engineering_facts::BuildTargetId,
    ) -> Option<&BuildTargetFact> {
        self.model.build_target(id)
    }
    pub fn dependency(
        &self,
        id: &crate::engineering_facts::DependencyId,
    ) -> Option<&DependencyFact> {
        self.model.dependency(id)
    }
    pub fn relationship(
        &self,
        id: &crate::engineering_facts::RelationshipId,
    ) -> Option<&RelationshipFact> {
        self.model.relationship(id)
    }
    pub fn reference(&self, id: &crate::engineering_facts::ReferenceId) -> Option<&ReferenceFact> {
        self.model.reference(id)
    }
    pub fn diagnostic(
        &self,
        id: &crate::engineering_facts::DiagnosticId,
    ) -> Option<&DiagnosticFact> {
        self.model.diagnostic(id)
    }
    pub fn architecture_rule(
        &self,
        id: &crate::engineering_facts::ArchitectureRuleId,
    ) -> Option<&ArchitectureRuleFact> {
        self.model.architecture_rule(id)
    }

    /// Enumerate every fact in deterministic category order, allocating
    /// nothing.
    pub fn iter(&self) -> impl Iterator<Item = FactRef<'_>> {
        self.model
            .workspaces()
            .iter()
            .map(FactRef::Workspace)
            .chain(self.model.modules().iter().map(FactRef::Module))
            .chain(self.model.packages().iter().map(FactRef::Package))
            .chain(self.model.symbols().iter().map(FactRef::Symbol))
            .chain(self.model.tests().iter().map(FactRef::Test))
            .chain(self.model.build_targets().iter().map(FactRef::BuildTarget))
            .chain(self.model.dependencies().iter().map(FactRef::Dependency))
            .chain(self.model.relationships().iter().map(FactRef::Relationship))
            .chain(self.model.references().iter().map(FactRef::Reference))
            .chain(self.model.diagnostics().iter().map(FactRef::Diagnostic))
            .chain(
                self.model
                    .architecture_rules()
                    .iter()
                    .map(FactRef::ArchitectureRule),
            )
    }
}
