//! CodeBro change engine — guarded mutation seam.
//!
//! The `coding` module owns [`coding::change_engine::ChangeEngine`], the
//! single mutation seam behind the MCP `apply_change` tool: workspace-root
//! path boundary, no blind overwrite, unambiguous-match enforcement, and
//! stale-content protection between prepare and apply.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

// Path-compatibility re-exports for historical in-crate paths.
pub use codebro_core::{error, tools};

pub mod coding;

pub use coding::{ChangeEngine, PreparedChange};
