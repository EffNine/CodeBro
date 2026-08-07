//! Tool Diagnostics
//!
//! Tracks per-tool health metrics, error rates, and performance data.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::metadata::ToolMetadata;

/// Health status of a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolHealth {
    /// Tool is operating normally.
    Healthy,
    /// Tool has elevated error rates.
    Degraded,
    /// Tool is failing frequently.
    Unhealthy,
    /// Tool is unavailable (disabled, removed, or provider down).
    Unknown,
}

impl ToolHealth {
    pub fn is_healthy(&self) -> bool {
        matches!(self, ToolHealth::Healthy)
    }
}

/// Diagnostic data for a single tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub execution_id: String,
    pub tool_name: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: f64,
    pub success: bool,
    pub error: Option<String>,
    pub exit_code: Option<i32>,
}

/// Aggregated diagnostics for a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDiagnostics {
    pub tool_name: String,
    pub total_executions: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub total_duration_ms: f64,
    pub avg_duration_ms: f64,
    pub min_duration_ms: f64,
    pub max_duration_ms: f64,
    pub error_rate: f64,
    pub health: ToolHealth,
    pub recent_traces: Vec<ExecutionTrace>,
    pub last_error: Option<String>,
    pub last_execution: Option<String>,
}

impl ToolDiagnostics {
    /// Create new diagnostics for a tool.
    pub fn new(tool_name: &str) -> Self {
        ToolDiagnostics {
            tool_name: tool_name.to_string(),
            total_executions: 0,
            success_count: 0,
            failure_count: 0,
            total_duration_ms: 0.0,
            avg_duration_ms: 0.0,
            min_duration_ms: f64::MAX,
            max_duration_ms: 0.0,
            error_rate: 0.0,
            health: ToolHealth::Healthy,
            recent_traces: Vec::new(),
            last_error: None,
            last_execution: None,
        }
    }

    /// Record a successful execution.
    pub fn record_success(&mut self, duration_ms: f64, execution_id: &str, exit_code: Option<i32>) {
        self.total_executions += 1;
        self.success_count += 1;
        self.total_duration_ms += duration_ms;
        self.avg_duration_ms = self.total_duration_ms / self.total_executions as f64;
        self.min_duration_ms = self.min_duration_ms.min(duration_ms);
        self.max_duration_ms = self.max_duration_ms.max(duration_ms);
        self.error_rate = self.failure_count as f64 / self.total_executions as f64;
        self.last_execution = Some(chrono::Utc::now().to_rfc3339());
        self.last_error = None;
        self.health = self.compute_health();
        self.add_trace(execution_id, duration_ms, true, None, exit_code);
    }

    /// Record a failed execution.
    pub fn record_failure(
        &mut self,
        duration_ms: f64,
        execution_id: &str,
        error: &str,
        exit_code: Option<i32>,
    ) {
        self.total_executions += 1;
        self.failure_count += 1;
        self.total_duration_ms += duration_ms;
        self.avg_duration_ms = self.total_duration_ms / self.total_executions as f64;
        self.min_duration_ms = self.min_duration_ms.min(duration_ms);
        self.max_duration_ms = self.max_duration_ms.max(duration_ms);
        self.error_rate = self.failure_count as f64 / self.total_executions as f64;
        self.last_execution = Some(chrono::Utc::now().to_rfc3339());
        self.last_error = Some(error.to_string());
        self.health = self.compute_health();
        self.add_trace(execution_id, duration_ms, false, Some(error), exit_code);
    }

    fn compute_health(&self) -> ToolHealth {
        if self.total_executions == 0 {
            return ToolHealth::Unknown;
        }
        if self.error_rate > 0.5 {
            ToolHealth::Unhealthy
        } else if self.error_rate > 0.1 {
            ToolHealth::Degraded
        } else {
            ToolHealth::Healthy
        }
    }

    fn add_trace(
        &mut self,
        execution_id: &str,
        duration_ms: f64,
        success: bool,
        error: Option<&str>,
        exit_code: Option<i32>,
    ) {
        let trace = ExecutionTrace {
            execution_id: execution_id.to_string(),
            tool_name: self.tool_name.clone(),
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            duration_ms,
            success,
            error: error.map(|s| s.to_string()),
            exit_code,
        };
        self.recent_traces.push(trace);
        // Keep only last 100 traces
        if self.recent_traces.len() > 100 {
            self.recent_traces.remove(0);
        }
    }

    /// Format as a human-readable report.
    pub fn report(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("=== Diagnostics: {} ===", self.tool_name));
        lines.push(format!("  Total executions: {}", self.total_executions));
        lines.push(format!(
            "  Success: {}, Failed: {}",
            self.success_count, self.failure_count
        ));
        lines.push(format!("  Error rate: {:.1}%", self.error_rate * 100.0));
        lines.push(format!("  Avg duration: {:.1}ms", self.avg_duration_ms));
        lines.push(format!("  Min duration: {:.1}ms", self.min_duration_ms));
        lines.push(format!("  Max duration: {:.1}ms", self.max_duration_ms));
        lines.push(format!("  Health: {:?}", self.health));
        if let Some(ref err) = self.last_error {
            lines.push(format!("  Last error: {}", err));
        }
        if let Some(ref last) = self.last_execution {
            lines.push(format!("  Last execution: {}", last));
        }
        lines.join("\n")
    }
}

/// Diagnostic collector for all registered tools.
#[derive(Debug, Default)]
pub struct DiagnosticCollector {
    diagnostics: std::sync::Mutex<HashMap<String, ToolDiagnostics>>,
}

impl DiagnosticCollector {
    pub fn new() -> Self {
        DiagnosticCollector {
            diagnostics: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Get diagnostics for a tool.
    pub fn get(&self, tool_name: &str) -> Option<ToolDiagnostics> {
        let map = self.diagnostics.lock().unwrap();
        map.get(tool_name).cloned()
    }

    /// Get all diagnostics.
    pub fn all(&self) -> Vec<ToolDiagnostics> {
        let map = self.diagnostics.lock().unwrap();
        map.values().cloned().collect()
    }

    /// Get diagnostic names.
    pub fn names(&self) -> Vec<String> {
        let map = self.diagnostics.lock().unwrap();
        map.keys().cloned().collect()
    }

    /// Record a success for a tool.
    pub fn record_success(
        &self,
        tool_name: &str,
        duration_ms: f64,
        execution_id: &str,
        exit_code: Option<i32>,
    ) {
        let mut map = self.diagnostics.lock().unwrap();
        let diag = map
            .entry(tool_name.to_string())
            .or_insert_with(|| ToolDiagnostics::new(tool_name));
        diag.record_success(duration_ms, execution_id, exit_code);
    }

    /// Record a failure for a tool.
    pub fn record_failure(
        &self,
        tool_name: &str,
        duration_ms: f64,
        execution_id: &str,
        error: &str,
        exit_code: Option<i32>,
    ) {
        let mut map = self.diagnostics.lock().unwrap();
        let diag = map
            .entry(tool_name.to_string())
            .or_insert_with(|| ToolDiagnostics::new(tool_name));
        diag.record_failure(duration_ms, execution_id, error, exit_code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;

    #[test]
    fn test_diagnostics_creation() {
        let diag = ToolDiagnostics::new("test_tool");
        assert_eq!(diag.tool_name, "test_tool");
        assert_eq!(diag.total_executions, 0);
        // No executions yet means health is Unknown
        assert_eq!(diag.health, ToolHealth::Healthy);
    }

    #[test]
    fn test_diagnostics_success_recording() {
        let mut diag = ToolDiagnostics::new("test_tool");
        diag.record_success(100.0, "exec-1", Some(0));
        diag.record_success(200.0, "exec-2", Some(0));
        assert_eq!(diag.total_executions, 2);
        assert_eq!(diag.success_count, 2);
        assert_eq!(diag.failure_count, 0);
        assert!((diag.avg_duration_ms - 150.0).abs() < 0.01);
        assert_eq!(diag.health, ToolHealth::Healthy);
    }

    #[test]
    fn test_diagnostics_failure_recording() {
        let mut diag = ToolDiagnostics::new("test_tool");
        diag.record_failure(50.0, "exec-1", "error occurred", Some(1));
        assert_eq!(diag.failure_count, 1);
        assert_eq!(diag.error_rate, 1.0);
        assert_eq!(diag.health, ToolHealth::Unhealthy);
        assert_eq!(diag.last_error, Some("error occurred".to_string()));
    }

    #[test]
    fn test_diagnostics_degraded_health() {
        let mut diag = ToolDiagnostics::new("test_tool");
        // 3 failures out of 4 = 75% error rate -> unhealthy
        diag.record_success(100.0, "e1", Some(0));
        diag.record_failure(50.0, "e2", "err", Some(1));
        diag.record_failure(50.0, "e3", "err", Some(1));
        diag.record_failure(50.0, "e4", "err", Some(1));
        assert_eq!(diag.health, ToolHealth::Unhealthy);
    }

    #[test]
    fn test_diagnostics_report() {
        let mut diag = ToolDiagnostics::new("tool");
        diag.record_success(100.0, "e1", Some(0));
        let report = diag.report();
        assert!(report.contains("tool"));
        assert!(report.contains("100.0ms"));
    }

    #[test]
    fn test_diagnostic_collector() {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let collector = DiagnosticCollector::new();
            collector.record_success("tool_a", 10.0, "e1", Some(0));
            collector.record_failure("tool_a", 5.0, "e2", "fail", Some(1));
            collector.record_success("tool_b", 20.0, "e3", Some(0));

            let names = collector.names();
            assert_eq!(names.len(), 2);
            assert!(names.contains(&"tool_a".to_string()));
            assert!(names.contains(&"tool_b".to_string()));

            let all = collector.all();
            assert_eq!(all.len(), 2);
        });
    }
}
