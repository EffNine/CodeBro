//! Tool Platform Module
//!
//! The P3 Tool Platform provides a scalable architecture for managing tools
//! across built-in, external, MCP, and plugin providers.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                    ToolRegistry                         │
//! │  ┌──────────┐  ┌──────────┐  ┌──────────┐              │
//! │  │ Metadata │  │Lifecycle │  │ Hooks    │              │
//! │  └──────────┘  └──────────┘  └──────────┘              │
//! │  ┌──────────┐  ┌──────────┐  ┌──────────┐              │
//! │  │Diagnostics│ │Provider  │  │ Discovery│              │
//! │  └──────────┘  └──────────┘  └──────────┘              │
//! └─────────────────────────────────────────────────────────┘
//!                          │
//!          ┌───────────────┼───────────────┐
//!          ▼               ▼               ▼
//!    ┌───────────┐  ┌───────────┐  ┌───────────┐
//!    │ BuiltIn   │  │ MCP       │  │ Plugin    │
//!    │ Provider  │  │ Provider  │  │ Provider  │
//!    └───────────┘  └───────────┘  └───────────┘
//! ```
//!
//! # Core Concepts
//!
//! - **Tool**: A unit of work (reads files, runs commands, etc.)
//! - **ToolCapabilities**: Typed flags describing what a tool can do
//! - **ToolMetadata**: Rich, serializable metadata for each tool
//! - **ToolLifecycleState**: State machine for tool registration and enablement
//! - **ToolContext**: Execution context with workspace, session, and permissions
//! - **PermissionHook**: Pre-execution permission checks
//! - **RollbackHook**: Post-execution state tracking
//! - **AsyncTool**: Streaming output support
//! - **ToolDiagnostics**: Health and performance tracking
//! - **ToolProvider**: Abstraction for tool sources (built-in, MCP, plugin)
//! - **ToolDiscovery**: Finding available tools across providers

// Suppress clippy warnings for the new module structure
#![allow(dead_code, unused_imports, unused_variables, clippy::all)]

pub mod capabilities;
pub mod context;
pub mod diagnostics;
pub mod discovery;
pub mod hooks;
pub mod lifecycle;
pub mod metadata;
pub mod provider;
pub mod streaming;

// Existing tool modules
pub mod change;
pub mod executor;
pub mod filesystem;
pub mod git;
pub mod patch;
pub mod router;
pub mod shell;

// Re-export core types
pub use capabilities::{PermissionPolicy, ToolCapabilities, ToolCategory};
pub use context::{ExecutionId, ToolContext, ToolContextBuilder, ToolResult};
pub use diagnostics::{DiagnosticCollector, ExecutionTrace, ToolDiagnostics, ToolHealth};
pub use discovery::{DiscoveredTool, DiscoveryResult, ToolDiscovery};
pub use hooks::{
    CapabilityPermissionHook, DefaultRollbackHook, PermissionDecision, RollbackHook, ToolHooks,
};
pub use lifecycle::{LifecycleError, LifecycleManager, ToolLifecycleState};
pub use metadata::{ToolDefinition, ToolMetadata};
pub use provider::{BuiltInProvider, ProviderRegistry, ToolProvider};
pub use streaming::{channel_stream, sync_to_stream, AsyncTool, StreamChunk, StreamResult};

// Re-export existing types
pub use change::ChangePlan;
pub use executor::{detect_workspace_root, is_toolable, run_tool_pipeline};
pub use filesystem::{CreateFile, EditFile, ListFiles, ReadFile};
pub use git::{GitDiff, GitStatus};
pub use patch::{FilePatch, PatchEngine, PatchSet};
pub use router::{SmartToolRouter, ToolSelection};
pub use shell::{RunCommand, ShellCommandRecord, ShellHistory};

// Core trait
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn execute(&self, args: &str) -> anyhow::Result<String>;
}

/// Dispatcher for executing tools (re-exported from dispatcher).
pub use crate::dispatcher::ToolDispatcher;
/// Registry for managing tools (re-exported from dispatcher).
pub use crate::dispatcher::ToolRegistry;
