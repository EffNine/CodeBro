#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Store-level diagnostics (P10.5.1).
//!
//! [`FactDiagnostics`] produces a deterministic summary of a store's health:
//! fact/index counts, validation outcome and the content digest. There is no
//! telemetry, no wall clock and no counters — identical stores yield
//! identical diagnostics.

use serde::{Deserialize, Serialize};

use crate::fact_store::statistics::{IndexSizes, ReverseIndexSizes};
use crate::fact_store::store::FactStore;

/// A deterministic diagnostics summary of a [`FactStore`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FactDiagnosticsSummary {
    pub total_facts: usize,
    pub primary_index_entries: usize,
    pub reverse_index_entries: usize,
    pub validation_passed: bool,
    pub validation_issue_count: usize,
    pub snapshot_digest: String,
    pub index_sizes: IndexSizes,
    pub reverse_index_sizes: ReverseIndexSizes,
}

/// Collector for store-level diagnostics.
pub struct FactDiagnostics;

impl FactDiagnostics {
    /// Produce the deterministic diagnostics summary of a store.
    pub fn collect(store: &FactStore) -> FactDiagnosticsSummary {
        let index = store.index();
        let stats = store.statistics();
        let report = store.validate();
        FactDiagnosticsSummary {
            total_facts: store.len(),
            primary_index_entries: index.primary_len(),
            reverse_index_entries: index.reverse_len(),
            validation_passed: report.passed(),
            validation_issue_count: report.issue_count(),
            snapshot_digest: store.snapshot().digest().to_string(),
            index_sizes: stats.primary_index,
            reverse_index_sizes: stats.reverse_index,
        }
    }
}
