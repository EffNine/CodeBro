#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Fact Store Foundation (P10.5.1).
//!
//! The **canonical immutable repository for Engineering Facts**. The store
//! consumes the frozen `FactsModel` produced by the Engineering Facts Model
//! (P10.5.0) and exposes storage, deterministic indexing, lookup, query,
//! snapshot, statistics, diagnostics and validation boundaries.
//!
//! # Architecture Contract
//!
//! Fact Store **owns**:
//!
//! - [`FactCollection`] — the immutable fact repository.
//! - [`FactIndex`] — deterministic, read-only indexes.
//! - [`FactLookup`] — `O(log n)` id lookup surface.
//! - [`FactQuery`] — id/kind/scope queries (workspace, package, module,
//!   symbol), enumeration and filtering. No graph traversal.
//! - [`FactSnapshot`] — immutable snapshots, byte-identical for identical
//!   inputs, no timestamps, no randomness.
//! - [`FactStatistics`] — deterministic counts and sizes.
//! - [`FactDiagnostics`] — store-level diagnostics summary.
//! - [`FactValidation`] — duplicate facts, broken indexes, missing ids,
//!   orphan records and schema consistency.
//!
//! Fact Store does **not** own: graph construction, relationship traversal,
//! impact analysis, context compilation, parsing or workspace discovery.
//! Nothing in this module parses source, walks a graph or performs analysis;
//! queries are pure index projections over declared field references.
//!
//! # Construction
//!
//! Facts are inserted **only during construction** via [`FactStoreBuilder`]
//! (or by building a `FactsModel` first and passing it to
//! [`FactStore::build`]). After `build()` the store is frozen: no mutation
//! path, every index read-only. The store is `Clone`, `Send`, `Sync` and
//! safe to share behind `Arc`.
//!
//! # Determinism & Thread Safety
//!
//! All storage is sorted at build time; snapshots, statistics, diagnostics
//! and validation reports are byte-identical for identical inputs. There is
//! no UUID generation, no timestamp and no randomness anywhere in the
//! module. Every public type is `Send + Sync`.

pub mod collection;
pub mod diagnostics;
pub mod index;
pub mod lookup;
pub mod query;
pub mod snapshot;
pub mod statistics;
pub mod store;
pub mod validation;

#[cfg(test)]
mod tests;

pub use crate::fact_store::collection::FactCollection;
pub use crate::fact_store::diagnostics::{FactDiagnostics, FactDiagnosticsSummary};
pub use crate::fact_store::index::{FactIdPair, FactIndex, ReverseIndex};
pub use crate::fact_store::lookup::FactLookup;
pub use crate::fact_store::query::FactQuery;
pub use crate::fact_store::snapshot::FactSnapshot;
pub use crate::fact_store::statistics::{FactStatistics, IndexSizes, ReverseIndexSizes};
pub use crate::fact_store::store::{FactStore, FactStoreBuilder};
pub use crate::fact_store::validation::{
    FactValidation, FactValidationIssue, FactValidationReport, FactValidationRule,
};
