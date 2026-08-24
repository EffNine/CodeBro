//! Runtime tool support for the CodeBro MCP runtime.
//!
//! This is the SLIM live surface of the old tool platform. Only what the
//! production path needs survives here:
//!
//! - [`shell`] — policy-gated command execution (PTY-backed) and secret
//!   redaction; used by `sandbox` backends and MCP redaction.
//! - [`patch`] — `ChangePlan` / `PatchEngine`, the diff machinery behind the
//!   change engine (`coding::change_engine`).
//! - [`context`] / [`streaming`] / [`capabilities`] — shared execution types
//!   (`ToolContext`, streaming traits) used by both the live shell tool and
//!   the legacy registry via `legacy::tools`.
//!
//! The full historical tool platform (registry, hooks, lifecycle, discovery,
//! filesystem/git/playwright tools) lives in `crate::legacy::tools`.

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]

pub mod capabilities;
pub mod change;
pub mod context;
pub mod patch;
pub mod pty;
pub mod shell;
pub mod streaming;

// Re-export core types
pub use capabilities::{PermissionPolicy, ToolCapabilities, ToolCategory};
pub use context::{ExecutionId, ToolContext, ToolContextBuilder, ToolResult};
pub use patch::{FilePatch, PatchEngine, PatchSet};
pub use change::ChangePlan;
pub use shell::{RunCommand, ShellCommandRecord, ShellHistory};
pub use streaming::{
    channel_stream, channel_stream_factory, sync_to_stream, AsyncTool, StreamChunk, StreamResult,
};

/// Core tool trait.
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn execute(&self, args: &str) -> anyhow::Result<String>;

    /// If this tool supports streaming output (e.g. PTY-backed processes),
    /// return a handle to its [`AsyncTool`] implementation. The default is
    /// `None`; tools that stream override this. This is the single discovery
    /// seam the registry uses to route to [`AsyncTool`].
    fn as_async(&self) -> Option<&dyn AsyncTool> {
        None
    }
}
