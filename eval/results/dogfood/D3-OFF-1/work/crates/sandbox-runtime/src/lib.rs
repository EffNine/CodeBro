//! CodeBro sandbox runtime — isolated command execution.
//!
//! Trait-based execution abstraction with two backends: local PTY-backed
//! execution (policy-bounded) and OpenSandbox remote isolation. Execution
//! fails closed: when a configured backend is unavailable there is no silent
//! local fallback. Every execution returns structured evidence.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

// Path-compatibility re-exports for historical `crate::tools` / `crate::error`
// paths inside this crate.
pub use codebro_core::{error, tools};

pub mod sandbox;
