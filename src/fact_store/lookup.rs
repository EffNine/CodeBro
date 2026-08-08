#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! O(log n) lookup surface (P10.5.1).
//!
//! [`FactLookup`] borrows a [`FactStore`] and answers id lookups over the
//! collection (binary searches, zero allocation) plus index-backed scope
//! lookups (`facts_in_workspace`, `facts_in_package`, `facts_in_module`,
//! `facts_in_symbol`, `facts_of_kind`). It is a pure read view — nothing can
//! be mutated through it.

use crate::engineering_facts::{
    ArchitectureRuleFact, ArchitectureRuleId, BuildTargetFact, BuildTargetId, DependencyFact,
    DependencyId, DiagnosticFact, DiagnosticId, FactId, FactKind, FactRef, ModuleFact, ModuleId,
    PackageFact, PackageId, ReferenceFact, ReferenceId, RelationshipFact, RelationshipId,
    SymbolFact, SymbolId, TestFact, TestId, WorkspaceFact, WorkspaceId,
};
use crate::fact_store::index::{FactIdPair, FactIndex};
use crate::fact_store::store::FactStore;

/// Borrowed, read-only lookup surface over an immutable [`FactStore`].
#[derive(Debug, Clone, Copy)]
pub struct FactLookup<'a> {
    store: &'a FactStore,
}

impl<'a> FactLookup<'a> {
    pub(crate) fn new(store: &'a FactStore) -> Self {
        FactLookup { store }
    }

    fn index(&self) -> &'a FactIndex {
        self.store.index()
    }

    /// Locate any fact by union id.
    pub fn find(&self, id: &FactId) -> Option<FactRef<'a>> {
        self.store.collection().find(id)
    }

    /// True when any fact in the store carries `id`.
    pub fn contains(&self, id: &FactId) -> bool {
        self.store.collection().contains(id)
    }

    pub fn workspace(&self, id: &WorkspaceId) -> Option<&'a WorkspaceFact> {
        self.store.collection().workspace(id)
    }
    pub fn module(&self, id: &ModuleId) -> Option<&'a ModuleFact> {
        self.store.collection().module(id)
    }
    pub fn package(&self, id: &PackageId) -> Option<&'a PackageFact> {
        self.store.collection().package(id)
    }
    pub fn symbol(&self, id: &SymbolId) -> Option<&'a SymbolFact> {
        self.store.collection().symbol(id)
    }
    pub fn test(&self, id: &TestId) -> Option<&'a TestFact> {
        self.store.collection().test(id)
    }
    pub fn build_target(&self, id: &BuildTargetId) -> Option<&'a BuildTargetFact> {
        self.store.collection().build_target(id)
    }
    pub fn dependency(&self, id: &DependencyId) -> Option<&'a DependencyFact> {
        self.store.collection().dependency(id)
    }
    pub fn relationship(&self, id: &RelationshipId) -> Option<&'a RelationshipFact> {
        self.store.collection().relationship(id)
    }
    pub fn reference(&self, id: &ReferenceId) -> Option<&'a ReferenceFact> {
        self.store.collection().reference(id)
    }
    pub fn diagnostic(&self, id: &DiagnosticId) -> Option<&'a DiagnosticFact> {
        self.store.collection().diagnostic(id)
    }
    pub fn architecture_rule(&self, id: &ArchitectureRuleId) -> Option<&'a ArchitectureRuleFact> {
        self.store.collection().architecture_rule(id)
    }

    /// The complete sorted id list of a kind, via the primary index.
    pub fn facts_of_kind(&self, kind: FactKind) -> &'a [FactId] {
        self.index().facts_of_kind(kind)
    }

    /// O(log n) membership test over a primary index.
    pub fn contains_in_kind(&self, kind: FactKind, id: &FactId) -> bool {
        self.index().contains_in_kind(kind, id)
    }

    /// The sorted members of a workspace scope via the reverse index.
    pub fn facts_in_workspace(&self, id: &WorkspaceId) -> &'a [FactIdPair] {
        self.index().facts_in_workspace(id)
    }

    /// The sorted members of a package scope via the reverse index.
    pub fn facts_in_package(&self, id: &PackageId) -> &'a [FactIdPair] {
        self.index().facts_in_package(id)
    }

    /// The sorted members of a module scope via the reverse index.
    pub fn facts_in_module(&self, id: &ModuleId) -> &'a [FactIdPair] {
        self.index().facts_in_module(id)
    }

    /// The sorted members of a symbol scope via the reverse index.
    pub fn facts_in_symbol(&self, id: &SymbolId) -> &'a [FactIdPair] {
        self.index().facts_in_symbol(id)
    }
}
