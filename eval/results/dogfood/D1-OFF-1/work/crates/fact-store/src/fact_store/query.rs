#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Query surface (P10.5.1).
//!
//! [`FactQuery`] borrows a [`FactStore`] and answers: find by id, find by
//! kind, find by workspace, find by package, find by module, find by symbol,
//! enumerate and filter. Every query is a **pure index projection over
//! declared field references** — there is no graph traversal, no transitive
//! closure and no analysis anywhere on this surface. Filter results are
//! sorted and de-duplicated, keeping queries deterministic.

use crate::engineering_facts::{
    ArchitectureRuleId, FactId, FactKind, FactRef, ModuleId, PackageId, SymbolId, WorkspaceId,
};
use crate::fact_store::index::{fact_id_of, FactIdPair};
use crate::fact_store::store::FactStore;

/// Read-only query surface over an immutable [`FactStore`].
#[derive(Debug, Clone, Copy)]
pub struct FactQuery<'a> {
    store: &'a FactStore,
}

impl<'a> FactQuery<'a> {
    pub(crate) fn new(store: &'a FactStore) -> Self {
        FactQuery { store }
    }

    /// Find a single fact by union id.
    pub fn by_id(&self, id: &FactId) -> Option<FactRef<'a>> {
        self.store.collection().find(id)
    }

    /// Every id of a fact kind, sorted, via the primary index.
    pub fn by_kind(&self, kind: FactKind) -> &'a [FactId] {
        self.store.index().facts_of_kind(kind)
    }

    /// All facts scoped by a workspace, sorted, via the reverse index.
    pub fn by_workspace(&self, id: &WorkspaceId) -> &'a [FactIdPair] {
        self.store.index().facts_in_workspace(id)
    }

    /// All facts scoped by a package, sorted, via the reverse index.
    pub fn by_package(&self, id: &PackageId) -> &'a [FactIdPair] {
        self.store.index().facts_in_package(id)
    }

    /// All facts scoped by a module, sorted, via the reverse index.
    pub fn by_module(&self, id: &ModuleId) -> &'a [FactIdPair] {
        self.store.index().facts_in_module(id)
    }

    /// All facts about a symbol, sorted, via the reverse index.
    pub fn by_symbol(&self, id: &SymbolId) -> &'a [FactIdPair] {
        self.store.index().facts_in_symbol(id)
    }

    /// Enumerate every fact in deterministic category order, allocating
    /// nothing.
    pub fn enumerate(&self) -> impl Iterator<Item = FactRef<'a>> + 'a {
        self.store.collection().iter()
    }

    /// Filter the whole store by a predicate. Results are returned as a
    /// sorted, de-duplicated list of fact ids.
    pub fn filter<F>(&self, predicate: F) -> Vec<FactId>
    where
        F: Fn(&FactId, FactRef<'a>) -> bool,
    {
        let mut out: Vec<FactId> = Vec::new();
        for fact in self.store.collection().iter() {
            let id = fact_id_of(&fact);
            if predicate(&id, fact) {
                out.push(id);
            }
        }
        out.sort();
        out.dedup();
        out
    }
}
