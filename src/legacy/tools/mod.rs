//! LEGACY tool platform (pre-MCP TUI era).
//!
//! This module belongs to legacy architecture and is not part of the CodeBro
//! MCP Runtime. The live runtime keeps only `crate::tools::{shell, patch,
//! change, context, streaming, capabilities}`; everything else — registry,
//! hooks, lifecycle, discovery, filesystem/git/playwright tools, router,
//! executor — survives here solely for the legacy regression suite.
#![allow(dead_code, unused_imports, unused_variables, clippy::all)]

pub mod diagnostics;
pub mod discovery;
pub mod executor;
pub mod filesystem;
pub mod git;
pub mod hooks;
pub mod lifecycle;
pub mod metadata;
pub mod playwright;

pub mod provider;
pub mod router;

// Shared live infrastructure, re-exported under the same paths legacy code
// has always used (`super::context`, `crate::tools::shell`, ... resolve
// identically after the split).
pub use crate::tools::capabilities;
#[allow(unused_imports)]
pub use crate::tools::change;
pub use crate::tools::context;
pub use crate::tools::patch;
pub use crate::tools::pty;
pub use crate::tools::shell;
pub use crate::tools::streaming;

#[allow(unused_imports)]
pub use crate::tools::{AsyncTool,ChangePlan,ExecutionId,FilePatch,PatchEngine,PermissionPolicy,RunCommand,ShellCommandRecord,StreamChunk,StreamResult,Tool,ToolCapabilities,ToolCategory,ToolContext,ToolContextBuilder,ToolResult};

#[allow(unused_imports)]
pub use executor::{detect_workspace_root, is_toolable, run_tool_pipeline};
#[allow(unused_imports)]
pub use router::{SmartToolRouter, ToolSelection};
#[allow(unused_imports)]
pub use playwright::PlaywrightTool;
#[allow(unused_imports)]
pub use provider::{BuiltInProvider, ProviderRegistry, ToolProvider};
#[allow(unused_imports)]
pub use filesystem::{CreateFile, EditFile, ListFiles, ReadFile};
#[allow(unused_imports)]
pub use git::{GitDiff, GitStatus};
#[allow(unused_imports)]
pub use hooks::{CapabilityPermissionHook, DefaultRollbackHook, PermissionDecision, RollbackHook, ToolHooks};
#[allow(unused_imports)]
pub use metadata::{ToolDefinition, ToolMetadata};
#[allow(unused_imports)]
pub use diagnostics::{DiagnosticCollector, ExecutionTrace, ToolDiagnostics, ToolHealth};
#[allow(unused_imports)]
pub use lifecycle::{LifecycleError, LifecycleManager, ToolLifecycleState};
