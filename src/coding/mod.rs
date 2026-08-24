//! CodeBro MCP runtime — guarded mutation.
//!
//! The `coding` module owns the single mutation seam of the runtime: the
//! [`change_engine::ChangeEngine`] behind the MCP `apply_change` tool. It
//! enforces the workspace boundary, refuses blind overwrites and ambiguous
//! edits, and protects against stale content between prepare and apply.
//!
//! The historical Sprint 30F autonomous coding subagent (permission hook,
//! restricted registry, agent loop) is legacy architecture and lives in
//! `crate::legacy`.

pub mod change_engine;

pub use change_engine::{ChangeEngine, PreparedChange};
