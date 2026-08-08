#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! The immutable Fact Store aggregate (P10.5.1).
//!
//! [`FactStore`] is built once ([`FactStore::build`] or
//! [`FactStoreBuilder`]) and then frozen. Facts are inserted **only during
//! construction**; afterwards the store has no mutation path. It owns the
//! [`FactCollection`] and the read-only [`FactIndex`], and derives lookup,
//! query, snapshot, statistics, diagnostics and validation surfaces on
//! demand. The store is `Clone`, `Send`, `Sync` and shareable via `Arc`.

use crate::engineering_facts::{
    ArchitectureRuleFact, BuildTargetFact, DependencyFact, DiagnosticFact, FactsBuilder,
    FactsModel, ModuleFact, PackageFact, ReferenceFact, RelationshipFact, SymbolFact, TestFact,
    WorkspaceFact,
};
use crate::fact_store::collection::FactCollection;
use crate::fact_store::diagnostics::{FactDiagnostics, FactDiagnosticsSummary};
use crate::fact_store::index::FactIndex;
use crate::fact_store::lookup::FactLookup;
use crate::fact_store::query::FactQuery;
use crate::fact_store::snapshot::FactSnapshot;
use crate::fact_store::statistics::FactStatistics;
use crate::fact_store::validation::{FactValidation, FactValidationReport};

/// The canonical immutable repository of engineering facts.
#[derive(Debug, Clone, PartialEq)]
pub struct FactStore {
    collection: FactCollection,
    index: FactIndex,
}

impl Default for FactStore {
    fn default() -> Self {
        FactStore::empty()
    }
}

impl FactStore {
    /// An empty, immutable store.
    pub fn empty() -> Self {
        FactStore::build(FactsModel::empty())
    }

    /// Freeze a `FactsModel` into a fully indexed store. This is the sole
    /// construction path; the returned store has no mutation path.
    pub fn build(model: FactsModel) -> Self {
        let collection = FactCollection::from_model(model);
        let index = FactIndex::build(&collection);
        FactStore { collection, index }
    }

    /// Freeze a borrowed model into an immutable store (clones the facts at
    /// build time only).
    pub fn from_model(model: &FactsModel) -> Self {
        FactStore::build(model.clone())
    }

    /// Start a mutable builder that absorbs facts and freezes into an
    /// immutable store.
    pub fn builder() -> FactStoreBuilder {
        FactStoreBuilder::new()
    }

    /// The immutable fact repository.
    pub fn collection(&self) -> &FactCollection {
        &self.collection
    }

    /// The read-only deterministic index set.
    pub fn index(&self) -> &FactIndex {
        &self.index
    }

    /// O(log n) id lookup surface.
    pub fn lookup(&self) -> FactLookup<'_> {
        FactLookup::new(self)
    }

    /// Id/kind/scope query surface.
    pub fn query(&self) -> FactQuery<'_> {
        FactQuery::new(self)
    }

    /// Deterministic statistics over the store.
    pub fn statistics(&self) -> FactStatistics {
        FactStatistics::collect(self)
    }

    /// Deterministic store-level diagnostics.
    pub fn diagnostics(&self) -> FactDiagnosticsSummary {
        FactDiagnostics::collect(self)
    }

    /// Deterministic store validation report.
    pub fn validate(&self) -> FactValidationReport {
        FactValidation::validate(self)
    }

    /// An immutable, byte-identical snapshot of the store.
    pub fn snapshot(&self) -> FactSnapshot {
        FactSnapshot::capture(self)
    }

    /// Total number of facts.
    pub fn len(&self) -> usize {
        self.collection.len()
    }

    pub fn is_empty(&self) -> bool {
        self.collection.is_empty()
    }
}

/// Mutable absorbing builder for a [`FactStore`].
///
/// Facts are added in any order and `build()` sorts every category by id and
/// constructs the index set, freezing into an immutable store.
#[derive(Debug, Default)]
pub struct FactStoreBuilder {
    facts: FactsBuilder,
}

impl FactStoreBuilder {
    pub fn new() -> Self {
        FactStoreBuilder::default()
    }

    pub fn add_workspace(&mut self, fact: WorkspaceFact) -> &mut Self {
        self.facts.add_workspace(fact);
        self
    }

    pub fn add_module(&mut self, fact: ModuleFact) -> &mut Self {
        self.facts.add_module(fact);
        self
    }

    pub fn add_package(&mut self, fact: PackageFact) -> &mut Self {
        self.facts.add_package(fact);
        self
    }

    pub fn add_symbol(&mut self, fact: SymbolFact) -> &mut Self {
        self.facts.add_symbol(fact);
        self
    }

    pub fn add_test(&mut self, fact: TestFact) -> &mut Self {
        self.facts.add_test(fact);
        self
    }

    pub fn add_build_target(&mut self, fact: BuildTargetFact) -> &mut Self {
        self.facts.add_build_target(fact);
        self
    }

    pub fn add_dependency(&mut self, fact: DependencyFact) -> &mut Self {
        self.facts.add_dependency(fact);
        self
    }

    pub fn add_relationship(&mut self, fact: RelationshipFact) -> &mut Self {
        self.facts.add_relationship(fact);
        self
    }

    pub fn add_reference(&mut self, fact: ReferenceFact) -> &mut Self {
        self.facts.add_reference(fact);
        self
    }

    pub fn add_diagnostic(&mut self, fact: DiagnosticFact) -> &mut Self {
        self.facts.add_diagnostic(fact);
        self
    }

    pub fn add_architecture_rule(&mut self, fact: ArchitectureRuleFact) -> &mut Self {
        self.facts.add_architecture_rule(fact);
        self
    }

    /// Absorb every fact of an existing frozen model (clones at build time
    /// only).
    pub fn add_model(&mut self, model: &FactsModel) -> &mut Self {
        for f in model.workspaces() {
            self.facts.add_workspace(f.clone());
        }
        for f in model.modules() {
            self.facts.add_module(f.clone());
        }
        for f in model.packages() {
            self.facts.add_package(f.clone());
        }
        for f in model.symbols() {
            self.facts.add_symbol(f.clone());
        }
        for f in model.tests() {
            self.facts.add_test(f.clone());
        }
        for f in model.build_targets() {
            self.facts.add_build_target(f.clone());
        }
        for f in model.dependencies() {
            self.facts.add_dependency(f.clone());
        }
        for f in model.relationships() {
            self.facts.add_relationship(f.clone());
        }
        for f in model.references() {
            self.facts.add_reference(f.clone());
        }
        for f in model.diagnostics() {
            self.facts.add_diagnostic(f.clone());
        }
        for f in model.architecture_rules() {
            self.facts.add_architecture_rule(f.clone());
        }
        self
    }

    /// Freeze into an immutable, id-sorted store with full index coverage.
    pub fn build(self) -> FactStore {
        FactStore::build(self.facts.build())
    }
}

// ── Test support: stores with malformed indexes for validation tests ──────

#[cfg(test)]
impl FactStore {
    /// Assemble a store from an explicit collection/index pair so validation
    /// rules can be exercised against broken indexes.
    pub(crate) fn with_index_for_test(collection: FactCollection, index: FactIndex) -> FactStore {
        FactStore { collection, index }
    }
}
