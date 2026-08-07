#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Runtime context for CodeBro.
//!
//! `RuntimeContext` is a shared, immutable snapshot of the data that flows
//! through the runtime pipeline. It is constructed at the start of each
//! task and passed through every phase (observe, reason, synthesize, act).
//!
//! The context is intentionally read-only after construction to prevent
//! accidental mutation across phases. Consumers clone only the fields they
//! need.
//!
//! # Thread Safety
//!
//! All fields are `Clone + Send + Sync`. The context is cheap to clone
//! because the heavy data (tool_context, report) is wrapped in `Arc`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::reliability::{HealthMonitor, ResourceGuard, TimeoutManager};

/// Shared context passed through every phase of the runtime pipeline.
///
/// Constructed once per task via `RuntimeContext::new()` or
/// `RuntimeContext::empty()`.
#[derive(Debug, Clone)]
pub struct RuntimeContext {
    /// Unique task identifier.
    pub task_id: String,

    /// Correlation ID used by the reliability layer for tracing.
    pub correlation_id: String,

    /// The raw user request that initiated this task.
    pub user_request: String,

    /// Timestamp when the context was created.
    pub created_at: DateTime<Utc>,

    /// Ground-truth context gathered by the observe phase (tool pipeline).
    pub tool_context: Arc<Option<String>>,

    /// Report produced by the reason phase (coordinator / subagents).
    pub reason_report: Arc<Option<String>>,

    /// Partial response accumulated during synthesis.
    pub synthesized_response: Arc<String>,

    /// Number of tool-call loops completed so far.
    pub act_loop_count: u32,

    /// Maximum number of tool-call loops allowed.
    pub max_act_loops: u32,

    /// Timeout manager shared across pipeline phases.
    pub timeout_manager: TimeoutManager,

    /// Health monitor shared across pipeline phases.
    pub health_monitor: HealthMonitor,

    /// Resource guard shared across pipeline phases.
    pub resource_guard: ResourceGuard,

    /// Whether shutdown has been requested by the user.
    pub shutdown_requested: bool,
}

impl RuntimeContext {
    /// Creates a new context for the given user request.
    pub fn new(user_request: impl Into<String>) -> Self {
        RuntimeContext {
            task_id: Uuid::new_v4().to_string(),
            correlation_id: Uuid::new_v4().to_string(),
            user_request: user_request.into(),
            created_at: Utc::now(),
            tool_context: Arc::new(None),
            reason_report: Arc::new(None),
            synthesized_response: Arc::new(String::new()),
            act_loop_count: 0,
            max_act_loops: 5,
            timeout_manager: TimeoutManager::new(),
            health_monitor: HealthMonitor::new(),
            resource_guard: ResourceGuard::new(),
            shutdown_requested: false,
        }
    }

    /// Creates an empty context (used for testing).
    pub fn empty() -> Self {
        RuntimeContext {
            task_id: String::from("test-task"),
            correlation_id: String::from("test-corr"),
            user_request: String::new(),
            created_at: Utc::now(),
            tool_context: Arc::new(None),
            reason_report: Arc::new(None),
            synthesized_response: Arc::new(String::new()),
            act_loop_count: 0,
            max_act_loops: 5,
            timeout_manager: TimeoutManager::new(),
            health_monitor: HealthMonitor::new(),
            resource_guard: ResourceGuard::new(),
            shutdown_requested: false,
        }
    }

    /// Returns a new context with the tool context populated.
    pub fn with_tool_context(mut self, ctx: String) -> Self {
        self.tool_context = Arc::new(Some(ctx));
        self
    }

    /// Returns a new context with the reason report populated.
    pub fn with_reason_report(mut self, report: String) -> Self {
        self.reason_report = Arc::new(Some(report));
        self
    }

    /// Returns a new context with the synthesized response appended.
    pub fn with_synthesized_response(mut self, response: String) -> Self {
        *Arc::make_mut(&mut self.synthesized_response) = response;
        self
    }

    /// Returns the current act loop count.
    pub fn act_loop_count(&self) -> u32 {
        self.act_loop_count
    }

    /// Returns whether the act loop has reached its limit.
    pub fn is_act_loop_limit_reached(&self) -> bool {
        self.act_loop_count >= self.max_act_loops
    }

    /// Returns whether a shutdown has been requested.
    pub fn is_shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    /// Requests shutdown of the runtime.
    pub fn request_shutdown(&mut self) {
        self.shutdown_requested = true;
        self.resource_guard.request_shutdown();
    }

    /// Returns the task ID.
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// Returns the correlation ID.
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }

    /// Returns a summary string suitable for event emission.
    pub fn summary(&self) -> String {
        format!(
            "task={} corr={} loops={}/{}",
            &self.task_id[..8],
            &self.correlation_id[..8],
            self.act_loop_count,
            self.max_act_loops,
        )
    }
}

impl Default for RuntimeContext {
    fn default() -> Self {
        Self::new("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        let ctx = RuntimeContext::new("test task");
        assert!(!ctx.task_id.is_empty());
        assert!(!ctx.correlation_id.is_empty());
        assert_eq!(ctx.user_request, "test task");
        assert!(ctx.act_loop_count == 0);
        assert!(!ctx.is_shutdown_requested());
    }

    #[test]
    fn test_context_empty() {
        let ctx = RuntimeContext::empty();
        assert_eq!(ctx.task_id, "test-task");
        assert_eq!(ctx.user_request, "");
    }

    #[test]
    fn test_with_tool_context() {
        let ctx = RuntimeContext::new("task").with_tool_context("file contents".to_string());
        assert!(ctx.tool_context.as_ref().is_some());
        assert_eq!(ctx.tool_context.as_ref().as_ref().unwrap(), "file contents");
    }

    #[test]
    fn test_with_reason_report() {
        let ctx = RuntimeContext::new("task").with_reason_report("report".to_string());
        assert!(ctx.reason_report.as_ref().is_some());
        assert_eq!(ctx.reason_report.as_ref().as_ref().unwrap(), "report");
    }

    #[test]
    fn test_with_synthesized_response() {
        let ctx = RuntimeContext::new("task").with_synthesized_response("hello".to_string());
        assert_eq!(*ctx.synthesized_response, "hello");
    }

    #[test]
    fn test_act_loop_counting() {
        let mut ctx = RuntimeContext::new("task");
        assert!(!ctx.is_act_loop_limit_reached());
        ctx.act_loop_count = 5;
        assert!(ctx.is_act_loop_limit_reached());
    }

    #[test]
    fn test_shutdown_request() {
        let mut ctx = RuntimeContext::new("task");
        assert!(!ctx.is_shutdown_requested());
        ctx.request_shutdown();
        assert!(ctx.is_shutdown_requested());
        assert!(ctx.resource_guard.should_shutdown());
    }

    #[test]
    fn test_summary() {
        let ctx = RuntimeContext::new("task");
        let s = ctx.summary();
        assert!(s.contains("task="));
        assert!(s.contains("corr="));
        assert!(s.contains("loops=0/5"));
    }

    #[test]
    fn test_default() {
        let ctx = RuntimeContext::default();
        assert_eq!(ctx.user_request, "");
    }
}
