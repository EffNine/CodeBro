#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Diagnostics for the reliability layer.
///
/// Provides structured failure traces and recovery traces for post-mortem analysis.
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::reliability::error::RuntimeErrorCategory;

/// A trace of a single failure event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureTrace {
    /// Unique correlation ID for this trace.
    pub correlation_id: String,
    /// Timestamp of the failure.
    pub timestamp: String,
    /// The classified error category.
    pub category: RuntimeErrorCategory,
    /// The original error message.
    pub message: String,
    /// The source of the error (e.g., "provider:openai").
    pub source: String,
    /// The recovery action that was taken, if any.
    pub recovery_action: Option<String>,
    /// Whether the failure was recovered.
    pub recovered: bool,
}

/// A trace of a single recovery event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryTrace {
    /// Unique correlation ID matching the failure trace.
    pub correlation_id: String,
    /// Timestamp of the recovery.
    pub timestamp: String,
    /// The original error message.
    pub original_error: String,
    /// The action taken to recover.
    pub action_taken: String,
    /// Whether the recovery was successful.
    pub success: bool,
    /// How many retries were attempted.
    pub retry_count: u32,
}

/// Maximum number of traces to retain in memory.
const MAX_TRACES: usize = 1000;

/// Central diagnostics collector for the reliability layer.
///
/// Thread-safe: can be shared across tasks via `Arc`.
#[derive(Debug, Clone)]
pub struct Diagnostics {
    inner: Arc<Mutex<DiagnosticsInner>>,
}

#[derive(Debug)]
struct DiagnosticsInner {
    failure_traces: Vec<FailureTrace>,
    recovery_traces: Vec<RecoveryTrace>,
    current_correlation_id: String,
}

impl Diagnostics {
    /// Creates a new `Diagnostics` collector.
    pub fn new() -> Self {
        Diagnostics {
            inner: Arc::new(Mutex::new(DiagnosticsInner {
                failure_traces: Vec::new(),
                recovery_traces: Vec::new(),
                current_correlation_id: uuid::Uuid::new_v4().to_string(),
            })),
        }
    }

    /// Creates a new correlation ID for a new session/task.
    pub fn new_correlation_id(&self) -> String {
        let mut inner = self.inner.lock().unwrap();
        inner.current_correlation_id = uuid::Uuid::new_v4().to_string();
        inner.current_correlation_id.clone()
    }

    /// Returns the current correlation ID.
    pub fn correlation_id(&self) -> String {
        let inner = self.inner.lock().unwrap();
        inner.current_correlation_id.clone()
    }

    /// Records a failure trace.
    pub fn record_failure(
        &self,
        category: RuntimeErrorCategory,
        message: &str,
        source: &str,
        recovery_action: Option<&str>,
        recovered: bool,
    ) {
        let mut inner = self.inner.lock().unwrap();
        let trace = FailureTrace {
            correlation_id: inner.current_correlation_id.clone(),
            timestamp: chrono::Local::now().to_rfc3339(),
            category: category.clone(),
            message: message.to_string(),
            source: source.to_string(),
            recovery_action: recovery_action.map(|s| s.to_string()),
            recovered,
        };
        inner.failure_traces.push(trace);
        if inner.failure_traces.len() > MAX_TRACES {
            inner.failure_traces.remove(0);
        }
    }

    /// Records a recovery trace.
    pub fn record_recovery(
        &self,
        original_error: &str,
        action_taken: &str,
        success: bool,
        retry_count: u32,
    ) {
        let mut inner = self.inner.lock().unwrap();
        let trace = RecoveryTrace {
            correlation_id: inner.current_correlation_id.clone(),
            timestamp: chrono::Local::now().to_rfc3339(),
            original_error: original_error.to_string(),
            action_taken: action_taken.to_string(),
            success,
            retry_count,
        };
        inner.recovery_traces.push(trace);
        if inner.recovery_traces.len() > MAX_TRACES {
            inner.recovery_traces.remove(0);
        }
    }

    /// Returns all failure traces.
    pub fn failure_traces(&self) -> Vec<FailureTrace> {
        let inner = self.inner.lock().unwrap();
        inner.failure_traces.clone()
    }

    /// Returns all recovery traces.
    pub fn recovery_traces(&self) -> Vec<RecoveryTrace> {
        let inner = self.inner.lock().unwrap();
        inner.recovery_traces.clone()
    }

    /// Returns the count of recorded failures.
    pub fn failure_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.failure_traces.len()
    }

    /// Returns the count of recorded recoveries.
    pub fn recovery_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.recovery_traces.len()
    }

    /// Returns the number of recovered failures.
    pub fn recovered_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.failure_traces.iter().filter(|t| t.recovered).count()
    }

    /// Returns the number of unrecovered failures.
    pub fn unrecovered_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.failure_traces.iter().filter(|t| !t.recovered).count()
    }

    /// Returns failure traces for a specific category.
    pub fn failures_by_category(&self, category: &RuntimeErrorCategory) -> Vec<FailureTrace> {
        let inner = self.inner.lock().unwrap();
        inner
            .failure_traces
            .iter()
            .filter(|t| &t.category == category)
            .cloned()
            .collect()
    }

    /// Clears all traces.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.failure_traces.clear();
        inner.recovery_traces.clear();
    }

    /// Returns a summary of the diagnostics.
    pub fn summary(&self) -> String {
        let inner = self.inner.lock().unwrap();
        format!(
            "Diagnostics Summary:\n  Correlation ID: {}\n  Failure traces: {}\n  Recovery traces: {}\n  Recovered: {}\n  Unrecovered: {}",
            inner.current_correlation_id,
            inner.failure_traces.len(),
            inner.recovery_traces.len(),
            inner.failure_traces.iter().filter(|t| t.recovered).count(),
            inner.failure_traces.iter().filter(|t| !t.recovered).count(),
        )
    }
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_empty() {
        let diag = Diagnostics::new();
        assert_eq!(diag.failure_count(), 0);
        assert_eq!(diag.recovery_count(), 0);
        assert_eq!(diag.recovered_count(), 0);
        assert_eq!(diag.unrecovered_count(), 0);
    }

    #[test]
    fn test_record_failure() {
        let diag = Diagnostics::new();
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "request timed out",
            "provider:openai",
            None,
            false,
        );
        assert_eq!(diag.failure_count(), 1);
        assert_eq!(diag.unrecovered_count(), 1);

        let traces = diag.failure_traces();
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].category, RuntimeErrorCategory::ProviderTimeout);
        assert_eq!(traces[0].message, "request timed out");
        assert_eq!(traces[0].source, "provider:openai");
        assert!(!traces[0].recovered);
    }

    #[test]
    fn test_record_recovery() {
        let diag = Diagnostics::new();
        diag.record_recovery("timeout error", "retry", true, 1);
        assert_eq!(diag.recovery_count(), 1);

        let traces = diag.recovery_traces();
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].original_error, "timeout error");
        assert_eq!(traces[0].action_taken, "retry");
        assert!(traces[0].success);
        assert_eq!(traces[0].retry_count, 1);
    }

    #[test]
    fn test_correlation_id() {
        let diag = Diagnostics::new();
        let id1 = diag.correlation_id();
        assert!(!id1.is_empty());

        let id2 = diag.new_correlation_id();
        assert_ne!(id1, id2);
        assert_eq!(diag.correlation_id(), id2);
    }

    #[test]
    fn test_lru_eviction() {
        let diag = Diagnostics::new();
        // Record more than MAX_TRACES failures
        for i in 0..=MAX_TRACES {
            diag.record_failure(
                RuntimeErrorCategory::Unknown,
                &format!("error {}", i),
                "test",
                None,
                false,
            );
        }
        assert_eq!(diag.failure_count(), MAX_TRACES);
    }

    #[test]
    fn test_failures_by_category() {
        let diag = Diagnostics::new();
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "t1",
            "p",
            None,
            false,
        );
        diag.record_failure(RuntimeErrorCategory::ToolTimeout, "t2", "tool", None, false);
        diag.record_failure(
            RuntimeErrorCategory::ProviderTimeout,
            "t3",
            "p",
            None,
            false,
        );

        let timeouts = diag.failures_by_category(&RuntimeErrorCategory::ProviderTimeout);
        assert_eq!(timeouts.len(), 2);

        let tool_timeouts = diag.failures_by_category(&RuntimeErrorCategory::ToolTimeout);
        assert_eq!(tool_timeouts.len(), 1);
    }

    #[test]
    fn test_clear() {
        let diag = Diagnostics::new();
        diag.record_failure(RuntimeErrorCategory::Unknown, "err", "src", None, false);
        diag.record_recovery("err", "retry", true, 1);
        assert_eq!(diag.failure_count(), 1);
        assert_eq!(diag.recovery_count(), 1);

        diag.clear();
        assert_eq!(diag.failure_count(), 0);
        assert_eq!(diag.recovery_count(), 0);
    }

    #[test]
    fn test_summary() {
        let diag = Diagnostics::new();
        diag.record_failure(RuntimeErrorCategory::Unknown, "err", "src", None, false);
        diag.record_failure(RuntimeErrorCategory::Unknown, "err2", "src", None, true);
        let summary = diag.summary();
        assert!(summary.contains("Failure traces: 2"));
        assert!(summary.contains("Recovered: 1"));
        assert!(summary.contains("Unrecovered: 1"));
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let diag = Diagnostics::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let diag = diag.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        diag.record_failure(
                            RuntimeErrorCategory::Unknown,
                            &format!("err {}", i),
                            "src",
                            None,
                            false,
                        );
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(diag.failure_count(), 1000);
    }
}
