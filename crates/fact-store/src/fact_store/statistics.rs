#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Deterministic store statistics (P10.5.1).
//!
//! [`FactStatistics`] captures the fact counts, per-kind primary index sizes,
//! reverse index sizes and the deterministic snapshot digest of a store.
//! Everything is derived from the frozen store — there are no counters,
//! timestamps or randomness.

use serde::{Deserialize, Serialize};

use crate::engineering_facts::{FactKind, ModelCounts};
use crate::fact_store::store::FactStore;

/// Primary index sizes per entity kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IndexSizes {
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

/// Reverse index sizes per scope dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReverseIndexSizes {
    pub by_workspace: usize,
    pub by_package: usize,
    pub by_module: usize,
    pub by_symbol: usize,
    pub total: usize,
}

/// Deterministic statistics over a [`FactStore`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FactStatistics {
    pub total_facts: usize,
    pub counts: ModelCounts,
    pub primary_index: IndexSizes,
    pub reverse_index: ReverseIndexSizes,
    /// Deterministic content digest of the store's snapshot.
    pub snapshot_digest: String,
}

impl FactStatistics {
    /// Collect statistics over a frozen store.
    pub fn collect(store: &FactStore) -> Self {
        let index = store.index();
        let primary = IndexSizes {
            workspaces: index.kind_len(FactKind::Workspace),
            modules: index.kind_len(FactKind::Module),
            packages: index.kind_len(FactKind::Package),
            symbols: index.kind_len(FactKind::Symbol),
            tests: index.kind_len(FactKind::Test),
            build_targets: index.kind_len(FactKind::BuildTarget),
            dependencies: index.kind_len(FactKind::Dependency),
            relationships: index.kind_len(FactKind::Relationship),
            references: index.kind_len(FactKind::Reference),
            diagnostics: index.kind_len(FactKind::Diagnostic),
            architecture_rules: index.kind_len(FactKind::ArchitectureRule),
            total: index.primary_len(),
        };
        let reverse = ReverseIndexSizes {
            by_workspace: index.reverse_workspace().len(),
            by_package: index.reverse_package().len(),
            by_module: index.reverse_module().len(),
            by_symbol: index.reverse_symbol().len(),
            total: index.reverse_len(),
        };
        FactStatistics {
            total_facts: store.len(),
            counts: store.collection().counts(),
            primary_index: primary,
            reverse_index: reverse,
            snapshot_digest: store.snapshot().digest().to_string(),
        }
    }

    /// Number of facts of a given kind.
    pub fn count_by_kind(&self, kind: FactKind) -> usize {
        match kind {
            FactKind::Workspace => self.counts.workspaces,
            FactKind::Module => self.counts.modules,
            FactKind::Package => self.counts.packages,
            FactKind::Symbol => self.counts.symbols,
            FactKind::Test => self.counts.tests,
            FactKind::BuildTarget => self.counts.build_targets,
            FactKind::Dependency => self.counts.dependencies,
            FactKind::Relationship => self.counts.relationships,
            FactKind::Reference => self.counts.references,
            FactKind::Diagnostic => self.counts.diagnostics,
            FactKind::ArchitectureRule => self.counts.architecture_rules,
        }
    }
}
