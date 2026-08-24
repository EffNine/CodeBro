//! CodeBro indexer — repository intelligence pipeline (`codebro init`).
//!
//! Scans a workspace (manifest detection + tree-sitter parsing via the
//! parsers crate), builds cross-module relationship/reference facts, and
//! freezes results into `.codebro/facts.json`.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

// Path-compatibility re-exports for historical in-crate paths.
pub use codebro_core::{error, persistence, provenance};
pub use codebro_fact_store::{engineering_facts, fact_store};
pub use codebro_identity_runtime::project_identity;
pub use codebro_impact_engine::impact;
pub use codebro_parsers::intelligence;

pub mod init;
