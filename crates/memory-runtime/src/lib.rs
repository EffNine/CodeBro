//! CodeBro memory runtime — persistent engineering memory.
//!
//! `memory_runtime` is the generic bounded-memory runtime (tiers, budgets,
//! excerpts); `engineering_memory` is the CodeBro-specific store over it.
//! Agent-recorded memory is low-trust by definition and is never promoted
//! into the verified fact store.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

// Path-compatibility re-exports: historical `crate::project_identity`,
// `crate::persistence`, `crate::error` paths keep resolving inside this crate.
pub use codebro_core::{error, persistence};
pub use codebro_identity_runtime::project_identity;

pub mod engineering_memory;
pub mod memory_runtime;
