//! CodeBro core — shared runtime primitives.
//!
//! Everything here has no dependency on higher-level engineering domains:
//!
//! - [`error`] — canonical error type
//! - [`provenance`] — evidence provenance model for executions and facts
//! - [`repo_state`] — git repository state capture (`RepoState`)
//! - [`persistence`] — atomic write + quarantine helpers
//! - [`cancellation`] — cooperative cancellation primitives
//! - [`config`] — configuration loading/persistence
//! - [`tools`] — execution infrastructure: policy-gated shell, patch engine,
//!   PTY support, tool context/streaming/capability types

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

pub mod cancellation;
pub mod config;
pub mod error;
pub mod persistence;
pub mod provenance;
pub mod repo_state;
pub mod tools;

pub use repo_state::{RepoIdentity, RepoState};
