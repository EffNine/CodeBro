//! Tool Execution Context
//!
//! Carries workspace, session, and permission information during tool execution.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use super::capabilities::ToolCapabilities;

/// Identifier for a tool execution session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExecutionId(pub String);

impl ExecutionId {
    pub fn new() -> Self {
        ExecutionId(Uuid::new_v4().to_string())
    }
}

impl Default for ExecutionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ExecutionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The execution context passed to every tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContext {
    /// Unique execution ID.
    pub execution_id: ExecutionId,
    /// Session ID (links to user session).
    pub session_id: Option<String>,
    /// Workspace root path.
    pub workspace_root: Option<PathBuf>,
    /// Current working directory.
    pub working_directory: Option<PathBuf>,
    /// Tool name being executed.
    pub tool_name: String,
    /// Tool capabilities (from metadata).
    pub tool_capabilities: ToolCapabilities,
    /// User-provided arguments.
    pub args: String,
    /// Whether this execution requires confirmation.
    pub requires_confirmation: bool,
    /// Correlation ID for tracing across modules.
    pub correlation_id: String,
}

impl ToolContext {
    /// Create a new tool context.
    pub fn new(tool_name: &str, args: &str) -> Self {
        ToolContext {
            execution_id: ExecutionId::new(),
            session_id: None,
            workspace_root: None,
            working_directory: None,
            tool_name: tool_name.to_string(),
            tool_capabilities: ToolCapabilities::default(),
            args: args.to_string(),
            requires_confirmation: false,
            correlation_id: Uuid::new_v4().to_string(),
        }
    }

    /// Create a builder for fluent context construction.
    pub fn builder(tool_name: &str, args: &str) -> ToolContextBuilder {
        ToolContextBuilder::new(tool_name, args)
    }

    /// Check if this context requires user confirmation.
    pub fn needs_confirmation(&self) -> bool {
        self.requires_confirmation || self.tool_capabilities.requires_confirmation
    }

    /// Check if the tool is mutating.
    pub fn is_mutating(&self) -> bool {
        self.tool_capabilities.is_mutating()
    }
}

/// Builder for `ToolContext`.
#[derive(Debug)]
pub struct ToolContextBuilder {
    context: ToolContext,
}

impl ToolContextBuilder {
    pub fn new(tool_name: &str, args: &str) -> Self {
        ToolContextBuilder {
            context: ToolContext::new(tool_name, args),
        }
    }

    pub fn with_session_id(mut self, id: &str) -> Self {
        self.context.session_id = Some(id.to_string());
        self
    }

    pub fn with_workspace_root(mut self, root: PathBuf) -> Self {
        self.context.workspace_root = Some(root);
        self
    }

    pub fn with_working_directory(mut self, dir: PathBuf) -> Self {
        self.context.working_directory = Some(dir);
        self
    }

    pub fn with_capabilities(mut self, caps: ToolCapabilities) -> Self {
        self.context.tool_capabilities = caps;
        self
    }

    pub fn with_correlation_id(mut self, id: &str) -> Self {
        self.context.correlation_id = id.to_string();
        self
    }

    pub fn build(self) -> ToolContext {
        self.context
    }
}

/// Result of a tool execution, carrying timing and diagnostic info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// The execution context used.
    pub context: ToolContext,
    /// Whether execution succeeded.
    pub success: bool,
    /// Output text (success) or error message (failure).
    pub output: String,
    /// Execution duration in milliseconds.
    pub duration_ms: f64,
    /// Exit code (if applicable).
    pub exit_code: Option<i32>,
    /// Optional diagnostic trace.
    pub trace: Option<String>,
}

impl ToolResult {
    /// Create a successful result.
    pub fn success(context: ToolContext, output: String, duration_ms: f64) -> Self {
        ToolResult {
            context,
            success: true,
            output,
            duration_ms,
            exit_code: Some(0),
            trace: None,
        }
    }

    /// Create a failed result.
    pub fn failure(context: ToolContext, error: String, duration_ms: f64) -> Self {
        ToolResult {
            context,
            success: false,
            output: error,
            duration_ms,
            exit_code: Some(-1),
            trace: None,
        }
    }

    /// Format as a human-readable summary.
    pub fn summary(&self) -> String {
        let status = if self.success { "OK" } else { "FAIL" };
        format!(
            "[{}] {} ({:.1}ms)",
            status, self.context.tool_name, self.duration_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        let ctx = ToolContext::new("read_file", "/path/to/file");
        assert_eq!(ctx.tool_name, "read_file");
        assert_eq!(ctx.args, "/path/to/file");
        assert_eq!(ctx.execution_id.0.len(), 36); // UUID
    }

    #[test]
    fn test_context_builder() {
        let ctx = ToolContext::builder("run_command", "echo hi")
            .with_session_id("session-1")
            .with_workspace_root(PathBuf::from("/workspace"))
            .with_capabilities(ToolCapabilities {
                executes_commands: true,
                ..Default::default()
            })
            .build();

        assert_eq!(ctx.session_id, Some("session-1".to_string()));
        assert_eq!(ctx.workspace_root, Some(PathBuf::from("/workspace")));
        assert!(ctx.tool_capabilities.executes_commands);
    }

    #[test]
    fn test_needs_confirmation() {
        let ctx = ToolContext::new("run_command", "rm -rf /");
        assert!(!ctx.needs_confirmation());

        let ctx = ToolContext::builder("run_command", "rm -rf /")
            .with_capabilities(ToolCapabilities {
                executes_commands: true,
                requires_confirmation: true,
                ..Default::default()
            })
            .build();
        assert!(ctx.needs_confirmation());
    }

    #[test]
    fn test_tool_result() {
        let ctx = ToolContext::new("test", "args");
        let result = ToolResult::success(ctx.clone(), "output".to_string(), 42.5);
        assert!(result.success);
        assert!((result.duration_ms - 42.5).abs() < 0.01);
        assert_eq!(result.summary(), "[OK] test (42.5ms)");

        let fail = ToolResult::failure(ctx, "error".to_string(), 10.0);
        assert!(!fail.success);
        assert_eq!(fail.summary(), "[FAIL] test (10.0ms)");
    }

    #[test]
    fn test_execution_id_uniqueness() {
        let id1 = ExecutionId::new();
        let id2 = ExecutionId::new();
        assert_ne!(id1.0, id2.0);
    }
}
