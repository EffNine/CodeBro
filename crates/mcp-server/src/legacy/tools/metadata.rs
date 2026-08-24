//! Tool Metadata
//!
//! Rich, serializable metadata for each registered tool.
//! Used by the TUI, agent router, and diagnostics subsystems.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::capabilities::{ToolCapabilities, ToolCategory};

/// Rich metadata describing a tool beyond name and description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    /// Unique identifier for this tool registration.
    pub id: String,
    /// Tool name (used for dispatch).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Semantic version of the tool implementation.
    pub version: String,
    /// Tool capabilities (read, write, execute, etc.).
    pub capabilities: ToolCapabilities,
    /// Derived category based on capabilities.
    pub category: ToolCategory,
    /// Provider that owns this tool (e.g., "builtin", "mcp", "plugin").
    pub provider: String,
    /// Whether this tool is deprecated.
    pub deprecated: bool,
    /// Optional deprecation note explaining why and what to use instead.
    pub deprecation_note: Option<String>,
    /// Search tags for tool discovery.
    pub tags: Vec<String>,
    /// Example usage strings.
    pub examples: Vec<String>,
    /// Number of times this tool has been executed.
    pub usage_count: u64,
    /// Number of successful executions.
    pub success_count: u64,
    /// Number of failed executions.
    pub failure_count: u64,
    /// Average execution time in milliseconds.
    pub avg_execution_ms: f64,
    /// Last execution timestamp (RFC 3339).
    pub last_used: Option<String>,
}

impl ToolMetadata {
    /// Create new metadata for a tool.
    pub fn new(
        name: &str,
        description: &str,
        capabilities: ToolCapabilities,
        provider: &str,
    ) -> Self {
        let category = ToolCategory::from_capabilities(&capabilities);
        ToolMetadata {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            description: description.to_string(),
            version: "1.0.0".to_string(),
            capabilities,
            category,
            provider: provider.to_string(),
            deprecated: false,
            deprecation_note: None,
            tags: Vec::new(),
            examples: Vec::new(),
            usage_count: 0,
            success_count: 0,
            failure_count: 0,
            avg_execution_ms: 0.0,
            last_used: None,
        }
    }

    /// Record a successful execution.
    pub fn record_success(&mut self, duration_ms: f64) {
        self.usage_count += 1;
        self.success_count += 1;
        self.last_used = Some(chrono::Utc::now().to_rfc3339());
        self.avg_execution_ms = if self.usage_count == 1 {
            duration_ms
        } else {
            let n = self.usage_count as f64;
            (self.avg_execution_ms * (n - 1.0) + duration_ms) / n
        };
    }

    /// Record a failed execution.
    pub fn record_failure(&mut self, duration_ms: f64) {
        self.usage_count += 1;
        self.failure_count += 1;
        self.last_used = Some(chrono::Utc::now().to_rfc3339());
        self.avg_execution_ms = if self.usage_count == 1 {
            duration_ms
        } else {
            let n = self.usage_count as f64;
            (self.avg_execution_ms * (n - 1.0) + duration_ms) / n
        };
    }

    /// Compute the success rate as a float between 0.0 and 1.0.
    pub fn success_rate(&self) -> f64 {
        if self.usage_count == 0 {
            return 1.0;
        }
        self.success_count as f64 / self.usage_count as f64
    }

    /// Check if the tool is usable (not deprecated and not removed).
    pub fn is_active(&self) -> bool {
        !self.deprecated
    }

    /// Format metadata as a human-readable summary.
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("{} v{}", self.name, self.version));
        lines.push(format!("  Category: {:?}", self.category));
        lines.push(format!("  Provider: {}", self.provider));
        lines.push(format!("  Capabilities: {}", self.capabilities.format()));
        if self.deprecated {
            lines.push(format!(
                "  WARNING: DEPRECATED - {}",
                self.deprecation_note.as_deref().unwrap_or("see note")
            ));
        }
        lines.push(format!(
            "  Usage: {} total, {} success, {} failed",
            self.usage_count, self.success_count, self.failure_count
        ));
        if self.avg_execution_ms > 0.0 {
            lines.push(format!("  Avg execution: {:.1}ms", self.avg_execution_ms));
        }
        lines.push(format!(
            "  Success rate: {:.1}%",
            self.success_rate() * 100.0
        ));
        lines.join("\n")
    }
}

/// A tool definition that can be used to create tool instances.
pub struct ToolDefinition {
    /// Tool metadata.
    pub metadata: ToolMetadata,
    /// Factory function that creates a new tool instance.
    pub factory: Box<dyn Fn() -> Box<dyn crate::tools::Tool> + Send + Sync>,
}

impl std::fmt::Debug for ToolDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolDefinition")
            .field("metadata", &self.metadata)
            .field("factory", &"<factory>")
            .finish()
    }
}

impl Clone for ToolDefinition {
    fn clone(&self) -> Self {
        // Factory closures are not cloneable; create a new definition instead.
        // This is intentionally a shallow clone that will fail if the factory is used after cloning.
        panic!("ToolDefinition cannot be cloned because the factory closure is not cloneable")
    }
}

impl ToolDefinition {
    /// Create a new tool definition.
    pub fn new(
        name: &str,
        description: &str,
        capabilities: ToolCapabilities,
        provider: &str,
        factory: impl Fn() -> Box<dyn crate::tools::Tool> + Send + Sync + 'static,
    ) -> Self {
        ToolDefinition {
            metadata: ToolMetadata::new(name, description, capabilities, provider),
            factory: Box::new(factory),
        }
    }

    /// Create a tool instance from this definition.
    pub fn create_tool(&self) -> Box<dyn crate::tools::Tool> {
        (self.factory)()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;

    struct DummyTool;
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "dummy"
        }
        fn description(&self) -> &str {
            "dummy"
        }
        fn execute(&self, _args: &str) -> anyhow::Result<String> {
            Ok("ok".to_string())
        }
    }

    #[test]
    fn test_metadata_creation() {
        let caps = ToolCapabilities {
            reads_files: true,
            ..Default::default()
        };
        let meta = ToolMetadata::new("test_tool", "A test tool", caps.clone(), "builtin");
        assert_eq!(meta.name, "test_tool");
        assert_eq!(meta.provider, "builtin");
        assert_eq!(meta.category, ToolCategory::Informational);
        assert_eq!(meta.usage_count, 0);
        assert!(meta.is_active());
    }

    #[test]
    fn test_metadata_recording() {
        let mut meta = ToolMetadata::new("t", "desc", ToolCapabilities::default(), "p");
        meta.record_success(100.0);
        meta.record_success(200.0);
        meta.record_failure(50.0);
        assert_eq!(meta.usage_count, 3);
        assert_eq!(meta.success_count, 2);
        assert_eq!(meta.failure_count, 1);
        assert!((meta.avg_execution_ms - 116.666).abs() < 1.0);
        assert!((meta.success_rate() - 2.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_metadata_deprecated() {
        let mut meta = ToolMetadata::new("old", "deprecated", ToolCapabilities::default(), "p");
        meta.deprecated = true;
        meta.deprecation_note = Some("Use new_tool instead".to_string());
        assert!(!meta.is_active());
        assert!(meta.summary().contains("DEPRECATED"));
    }

    #[test]
    fn test_tool_definition() {
        let def = ToolDefinition::new(
            "my_tool",
            "A tool",
            ToolCapabilities::default(),
            "builtin",
            || Box::new(DummyTool),
        );
        assert_eq!(def.metadata.name, "my_tool");
        let tool = def.create_tool();
        assert_eq!(tool.name(), "dummy");
    }

    #[test]
    fn test_empty_usage_rate() {
        let meta = ToolMetadata::new("t", "d", ToolCapabilities::default(), "p");
        assert_eq!(meta.success_rate(), 1.0);
    }
}
