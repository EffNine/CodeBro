#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Deterministic, read-only indexes (P10.5.1).
//!
//! [`FactIndex`] is built once from a frozen [`FactCollection`] and then
//! never mutated. It provides:
//!
//! - **Primary indexes** — the complete sorted id list of every entity kind
//!   (`WorkspaceId` … `ArchitectureRuleId`).
//! - **Reverse scope indexes** — workspace/package/module/symbol owners map
//!   to the sorted set of member fact ids. These are pure projections of
//!   *declared field references* (ownership fields and `SourceLocation`
//!   scope), never graph traversal.
//!
//! All indexes are `Vec`-backed and sorted, so lookups are `O(log n)` with
//! zero heap allocation and every index is byte-identical for identical
//! inputs.

use crate::engineering_facts::{FactId, FactKind, FactRef, IdKey};
use crate::fact_store::collection::FactCollection;
use serde::{Deserialize, Serialize};

/// A single reverse-index entry: `owner --contains--> member`. Both ends are
/// union fact ids. Stored sorted by `(owner, member)`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FactIdPair {
    pub owner: FactId,
    pub member: FactId,
}

/// A sorted, read-only reverse index.
///
/// Backed by a sorted `Vec<FactIdPair>`; `get` locates the owner's equal
/// range with a partition-point search (`O(log n)`) and returns a slice.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReverseIndex {
    entries: Vec<FactIdPair>,
}

impl ReverseIndex {
    /// Build a sorted reverse index from unsorted entries.
    pub(crate) fn new(mut entries: Vec<FactIdPair>) -> Self {
        entries.sort();
        entries.dedup();
        ReverseIndex { entries }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every entry, sorted by `(owner, member)`.
    pub fn entries(&self) -> &[FactIdPair] {
        &self.entries
    }

    /// The sorted entries owned by `owner`, as a contiguous slice. Returns
    /// an empty slice when the owner is unknown.
    pub fn get(&self, owner: &FactId) -> &[FactIdPair] {
        let start = self.entries.partition_point(|p| &p.owner < owner);
        let mut end = start;
        while end < self.entries.len() && &self.entries[end].owner == owner {
            end += 1;
        }
        &self.entries[start..end]
    }

    /// True when `owner` owns at least one member.
    pub fn contains_owner(&self, owner: &FactId) -> bool {
        !self.get(owner).is_empty()
    }
}

/// The deterministic, read-only index set of a [`FactCollection`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FactIndex {
    // Primary indexes: the complete sorted id list of every entity kind.
    workspaces: Vec<FactId>,
    modules: Vec<FactId>,
    packages: Vec<FactId>,
    symbols: Vec<FactId>,
    tests: Vec<FactId>,
    build_targets: Vec<FactId>,
    dependencies: Vec<FactId>,
    relationships: Vec<FactId>,
    references: Vec<FactId>,
    diagnostics: Vec<FactId>,
    architecture_rules: Vec<FactId>,
    // Reverse scope indexes.
    by_workspace: ReverseIndex,
    by_package: ReverseIndex,
    by_module: ReverseIndex,
    by_symbol: ReverseIndex,
}

impl FactIndex {
    /// Build the complete, sorted, read-only index set for a collection.
    pub fn build(collection: &FactCollection) -> FactIndex {
        FactIndex {
            workspaces: indexed(collection.workspaces(), |f| FactId::Workspace(f.id.clone())),
            modules: indexed(collection.modules(), |f| FactId::Module(f.id.clone())),
            packages: indexed(collection.packages(), |f| FactId::Package(f.id.clone())),
            symbols: indexed(collection.symbols(), |f| FactId::Symbol(f.id.clone())),
            tests: indexed(collection.tests(), |f| FactId::Test(f.id.clone())),
            build_targets: indexed(collection.build_targets(), |f| {
                FactId::BuildTarget(f.id.clone())
            }),
            dependencies: indexed(collection.dependencies(), |f| {
                FactId::Dependency(f.id.clone())
            }),
            relationships: indexed(collection.relationships(), |f| {
                FactId::Relationship(f.id.clone())
            }),
            references: indexed(collection.references(), |f| FactId::Reference(f.id.clone())),
            diagnostics: indexed(collection.diagnostics(), |f| {
                FactId::Diagnostic(f.id.clone())
            }),
            architecture_rules: indexed(collection.architecture_rules(), |f| {
                FactId::ArchitectureRule(f.id.clone())
            }),
            by_workspace: ReverseIndex::new(index_workspace(collection)),
            by_package: ReverseIndex::new(index_package(collection)),
            by_module: ReverseIndex::new(index_module(collection)),
            by_symbol: ReverseIndex::new(index_symbol(collection)),
        }
    }

    /// The complete, sorted id list of a fact kind. Empty for unknown ids.
    pub fn facts_of_kind(&self, kind: FactKind) -> &[FactId] {
        match kind {
            FactKind::Workspace => &self.workspaces,
            FactKind::Module => &self.modules,
            FactKind::Package => &self.packages,
            FactKind::Symbol => &self.symbols,
            FactKind::Test => &self.tests,
            FactKind::BuildTarget => &self.build_targets,
            FactKind::Dependency => &self.dependencies,
            FactKind::Relationship => &self.relationships,
            FactKind::Reference => &self.references,
            FactKind::Diagnostic => &self.diagnostics,
            FactKind::ArchitectureRule => &self.architecture_rules,
        }
    }

    /// O(log n) membership test over a primary index.
    pub fn contains_in_kind(&self, kind: FactKind, id: &FactId) -> bool {
        self.facts_of_kind(kind).binary_search(id).is_ok()
    }

    /// Size of the primary index of a kind.
    pub fn kind_len(&self, kind: FactKind) -> usize {
        self.facts_of_kind(kind).len()
    }

    /// Total primary index entries across all kinds.
    pub fn primary_len(&self) -> usize {
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

    /// Total reverse index entries.
    pub fn reverse_len(&self) -> usize {
        self.by_workspace.len()
            + self.by_package.len()
            + self.by_module.len()
            + self.by_symbol.len()
    }

    /// The sorted members of a workspace scope.
    pub fn facts_in_workspace(
        &self,
        owner: &crate::engineering_facts::WorkspaceId,
    ) -> &[FactIdPair] {
        self.by_workspace.get(&FactId::Workspace(owner.clone()))
    }

    /// The sorted members of a package scope.
    pub fn facts_in_package(&self, owner: &crate::engineering_facts::PackageId) -> &[FactIdPair] {
        self.by_package.get(&FactId::Package(owner.clone()))
    }

    /// The sorted members of a module scope.
    pub fn facts_in_module(&self, owner: &crate::engineering_facts::ModuleId) -> &[FactIdPair] {
        self.by_module.get(&FactId::Module(owner.clone()))
    }

    /// The sorted members of a symbol scope (facts about a symbol).
    pub fn facts_in_symbol(&self, owner: &crate::engineering_facts::SymbolId) -> &[FactIdPair] {
        self.by_symbol.get(&FactId::Symbol(owner.clone()))
    }

    /// The raw reverse workspace index (validation and diagnostics).
    pub fn reverse_workspace(&self) -> &ReverseIndex {
        &self.by_workspace
    }
    pub fn reverse_package(&self) -> &ReverseIndex {
        &self.by_package
    }
    pub fn reverse_module(&self) -> &ReverseIndex {
        &self.by_module
    }
    pub fn reverse_symbol(&self) -> &ReverseIndex {
        &self.by_symbol
    }
}

/// Convert an id-carrying slice into a sorted `Vec<FactId>`.
fn indexed<T, F>(facts: &[T], to_id: F) -> Vec<FactId>
where
    F: Fn(&T) -> FactId,
{
    let mut v: Vec<FactId> = facts.iter().map(to_id).collect();
    v.sort();
    v
}

/// Every fact scoped by a workspace: workspace-declared packages, packages
/// declaring the workspace, and any fact whose `SourceLocation.workspace`
/// references it.
fn index_workspace(collection: &FactCollection) -> Vec<FactIdPair> {
    let mut out = Vec::new();
    for w in collection.workspaces() {
        let owner = FactId::Workspace(w.id.clone());
        for p in &w.packages {
            out.push(pair(&owner, &FactId::Package(p.clone())));
        }
    }
    for p in collection.packages() {
        if let Some(w) = &p.workspace {
            out.push(pair(
                &FactId::Workspace(w.clone()),
                &FactId::Package(p.id.clone()),
            ));
        }
    }
    for f in collection.modules() {
        if let Some(w) = &f.location.workspace {
            out.push(pair(
                &FactId::Workspace(w.clone()),
                &FactId::Module(f.id.clone()),
            ));
        }
    }
    for f in collection.symbols() {
        if let Some(w) = &f.location.workspace {
            out.push(pair(
                &FactId::Workspace(w.clone()),
                &FactId::Symbol(f.id.clone()),
            ));
        }
    }
    for f in collection.relationships() {
        if let Some(w) = f.location.as_ref().and_then(|l| l.workspace.as_ref()) {
            out.push(pair(
                &FactId::Workspace(w.clone()),
                &FactId::Relationship(f.id.clone()),
            ));
        }
    }
    for f in collection.references() {
        if let Some(w) = f.location.as_ref().and_then(|l| l.workspace.as_ref()) {
            out.push(pair(
                &FactId::Workspace(w.clone()),
                &FactId::Reference(f.id.clone()),
            ));
        }
    }
    for f in collection.tests() {
        if let Some(w) = f.location.as_ref().and_then(|l| l.workspace.as_ref()) {
            out.push(pair(
                &FactId::Workspace(w.clone()),
                &FactId::Test(f.id.clone()),
            ));
        }
    }
    for f in collection.diagnostics() {
        if let Some(w) = f.location.as_ref().and_then(|l| l.workspace.as_ref()) {
            out.push(pair(
                &FactId::Workspace(w.clone()),
                &FactId::Diagnostic(f.id.clone()),
            ));
        }
    }
    out
}

/// Every fact scoped by a package: modules and build targets declaring the
/// package, plus any fact whose `SourceLocation.package` references it.
fn index_package(collection: &FactCollection) -> Vec<FactIdPair> {
    let mut out = Vec::new();
    for m in collection.modules() {
        if let Some(p) = &m.package {
            out.push(pair(
                &FactId::Package(p.clone()),
                &FactId::Module(m.id.clone()),
            ));
        }
    }
    for b in collection.build_targets() {
        if let Some(p) = &b.package {
            out.push(pair(
                &FactId::Package(p.clone()),
                &FactId::BuildTarget(b.id.clone()),
            ));
        }
    }
    for f in collection.modules() {
        if let Some(p) = &f.location.package {
            out.push(pair(
                &FactId::Package(p.clone()),
                &FactId::Module(f.id.clone()),
            ));
        }
    }
    for f in collection.symbols() {
        if let Some(p) = &f.location.package {
            out.push(pair(
                &FactId::Package(p.clone()),
                &FactId::Symbol(f.id.clone()),
            ));
        }
    }
    for f in collection.relationships() {
        if let Some(p) = f.location.as_ref().and_then(|l| l.package.as_ref()) {
            out.push(pair(
                &FactId::Package(p.clone()),
                &FactId::Relationship(f.id.clone()),
            ));
        }
    }
    for f in collection.references() {
        if let Some(p) = f.location.as_ref().and_then(|l| l.package.as_ref()) {
            out.push(pair(
                &FactId::Package(p.clone()),
                &FactId::Reference(f.id.clone()),
            ));
        }
    }
    for f in collection.tests() {
        if let Some(p) = f.location.as_ref().and_then(|l| l.package.as_ref()) {
            out.push(pair(
                &FactId::Package(p.clone()),
                &FactId::Test(f.id.clone()),
            ));
        }
    }
    for f in collection.diagnostics() {
        if let Some(p) = f.location.as_ref().and_then(|l| l.package.as_ref()) {
            out.push(pair(
                &FactId::Package(p.clone()),
                &FactId::Diagnostic(f.id.clone()),
            ));
        }
    }
    out
}

/// Every fact scoped by a module: symbols declaring the module, plus any
/// fact whose `SourceLocation.module` references it.
fn index_module(collection: &FactCollection) -> Vec<FactIdPair> {
    let mut out = Vec::new();
    for s in collection.symbols() {
        if let Some(m) = &s.module {
            out.push(pair(
                &FactId::Module(m.clone()),
                &FactId::Symbol(s.id.clone()),
            ));
        }
        if let Some(m) = &s.location.module {
            out.push(pair(
                &FactId::Module(m.clone()),
                &FactId::Symbol(s.id.clone()),
            ));
        }
    }
    for f in collection.relationships() {
        if let Some(m) = f.location.as_ref().and_then(|l| l.module.as_ref()) {
            out.push(pair(
                &FactId::Module(m.clone()),
                &FactId::Relationship(f.id.clone()),
            ));
        }
    }
    for f in collection.references() {
        if let Some(m) = f.location.as_ref().and_then(|l| l.module.as_ref()) {
            out.push(pair(
                &FactId::Module(m.clone()),
                &FactId::Reference(f.id.clone()),
            ));
        }
    }
    for f in collection.tests() {
        if let Some(m) = f.location.as_ref().and_then(|l| l.module.as_ref()) {
            out.push(pair(
                &FactId::Module(m.clone()),
                &FactId::Test(f.id.clone()),
            ));
        }
    }
    for f in collection.diagnostics() {
        if let Some(m) = f.location.as_ref().and_then(|l| l.module.as_ref()) {
            out.push(pair(
                &FactId::Module(m.clone()),
                &FactId::Diagnostic(f.id.clone()),
            ));
        }
    }
    out
}

/// Every fact about a symbol: tests exercising it, references and
/// relationships touching it, diagnostics related to it, modules exporting it
/// and architecture rules bounding it.
fn index_symbol(collection: &FactCollection) -> Vec<FactIdPair> {
    let mut out = Vec::new();
    for t in collection.tests() {
        for s in &t.tested {
            out.push(pair(
                &FactId::Symbol(s.clone()),
                &FactId::Test(t.id.clone()),
            ));
        }
    }
    for r in collection.references() {
        if matches!(r.target.kind(), FactKind::Symbol) {
            out.push(pair(&r.target.clone(), &FactId::Reference(r.id.clone())));
        }
        if matches!(r.referrer.kind(), FactKind::Symbol) {
            out.push(pair(&r.referrer.clone(), &FactId::Reference(r.id.clone())));
        }
    }
    for r in collection.relationships() {
        if matches!(r.source.kind(), FactKind::Symbol) {
            out.push(pair(&r.source.clone(), &FactId::Relationship(r.id.clone())));
        }
        if matches!(r.target.kind(), FactKind::Symbol) {
            out.push(pair(&r.target.clone(), &FactId::Relationship(r.id.clone())));
        }
    }
    for d in collection.diagnostics() {
        for related in &d.related {
            if matches!(related.kind(), FactKind::Symbol) {
                out.push(pair(related, &FactId::Diagnostic(d.id.clone())));
            }
        }
    }
    for m in collection.modules() {
        for e in &m.api.exports {
            out.push(pair(
                &FactId::Symbol(e.clone()),
                &FactId::Module(m.id.clone()),
            ));
        }
        for e in &m.api.entry_points {
            out.push(pair(
                &FactId::Symbol(e.clone()),
                &FactId::Module(m.id.clone()),
            ));
        }
    }
    for a in collection.architecture_rules() {
        if let Some(from) = &a.from {
            if matches!(from.kind(), FactKind::Symbol) {
                out.push(pair(from, &FactId::ArchitectureRule(a.id.clone())));
            }
        }
        if let Some(to) = &a.to {
            if matches!(to.kind(), FactKind::Symbol) {
                out.push(pair(to, &FactId::ArchitectureRule(a.id.clone())));
            }
        }
    }
    out
}

fn pair(owner: &FactId, member: &FactId) -> FactIdPair {
    FactIdPair {
        owner: owner.clone(),
        member: member.clone(),
    }
}

/// The union id of a borrowed fact reference.
pub(crate) fn fact_id_of(fact: &FactRef<'_>) -> FactId {
    match fact {
        FactRef::Workspace(f) => FactId::Workspace(f.id.clone()),
        FactRef::Module(f) => FactId::Module(f.id.clone()),
        FactRef::Package(f) => FactId::Package(f.id.clone()),
        FactRef::Symbol(f) => FactId::Symbol(f.id.clone()),
        FactRef::Test(f) => FactId::Test(f.id.clone()),
        FactRef::BuildTarget(f) => FactId::BuildTarget(f.id.clone()),
        FactRef::Dependency(f) => FactId::Dependency(f.id.clone()),
        FactRef::Relationship(f) => FactId::Relationship(f.id.clone()),
        FactRef::Reference(f) => FactId::Reference(f.id.clone()),
        FactRef::Diagnostic(f) => FactId::Diagnostic(f.id.clone()),
        FactRef::ArchitectureRule(f) => FactId::ArchitectureRule(f.id.clone()),
    }
}

// ── Test support: malformed indexes for validation tests ──────────────────

#[cfg(test)]
impl FactIndex {
    /// An index with a dangling reverse entry injected, for `BrokenIndex`
    /// validation coverage.
    pub(crate) fn with_broken_reverse_entry(collection: &FactCollection) -> FactIndex {
        let mut idx = FactIndex::build(collection);
        let mut entries = idx.by_symbol.entries().to_vec();
        entries.push(FactIdPair {
            owner: FactId::Symbol(crate::engineering_facts::SymbolId::new("ghost")),
            member: FactId::Symbol(crate::engineering_facts::SymbolId::new("ghost2")),
        });
        idx.by_symbol = ReverseIndex::new(entries);
        idx
    }

    /// An index whose symbol primary index carries a module id, for
    /// `SchemaMismatch` validation coverage.
    pub(crate) fn with_schema_mismatch(collection: &FactCollection) -> FactIndex {
        let mut idx = FactIndex::build(collection);
        idx.symbols
            .push(FactId::Module(crate::engineering_facts::ModuleId::new(
                "module-in-symbol-index",
            )));
        idx.symbols.sort();
        idx
    }

    /// An index whose symbol primary index omits the last symbol of the
    /// collection, for `MissingIds` validation coverage.
    pub(crate) fn with_missing_symbol(collection: &FactCollection) -> FactIndex {
        let mut idx = FactIndex::build(collection);
        if let Some(last) = collection.symbols().last() {
            let id = FactId::Symbol(last.id.clone());
            if let Ok(pos) = idx.symbols.binary_search(&id) {
                idx.symbols.remove(pos);
            }
        }
        idx
    }
}
