//! CodeBro impact engine — structural impact analysis.
//!
//! Answers "what breaks if this changes?" with descriptive evidence:
//! directed relationship edges (calls, imports, references), related tests,
//! owning module/package, and provenance. No risk scores, no prescriptions.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

// Path-compatibility re-exports for historical in-crate paths.
pub use codebro_core::{error, provenance};
pub use codebro_fact_store::{engineering_facts, fact_store};
pub use codebro_parsers::intelligence;

pub mod impact;
