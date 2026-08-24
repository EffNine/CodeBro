//! CodeBro fact store — canonical engineering facts + immutable store.
//!
//! `engineering_facts` is the canonical facts model (symbols, modules,
//! packages, tests, build targets, dependencies, relationships, references,
//! diagnostics, architecture rules). `fact_store` is the validated,
//! frozen-after-build store over that model.
//!
//! Path-compatibility re-exports keep historical `crate::error` /
//! `crate::provenance` paths resolving inside this crate.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

pub use codebro_core::{error, provenance};

pub mod engineering_facts;
pub mod fact_store;
