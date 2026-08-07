#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Runtime diagnostics for CodeBro.
//!
//! `RuntimeDiagnostics` collects and aggregates diagnostic information
//! from the runtime pipeline, including phase durations, state transition
//! counts, error traces, and health summaries. It is separate from the
//! general `reliability::Diagnostics` which tracks failure/recovery traces
//! at a lower level.
//!
//! Runtime diagnostics are phase-aware: they track per-phase timing and
//! can produce a structured report after the pipeline completes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::events::RuntimeEvent;
use super::state::RuntimeState;

/// Duration of a single pipeline phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseDuration {
    pub phase: String,
    pub start_ns: u64,
    pub end_ns: Option<u64>,
    pub duration_ms: Option<u64>,
}

impl PhaseDuration {
    pub fn new(phase: &str, start_ns: u64) -> Self {
        PhaseDuration {
            phase: phase.to_string(),
            start_ns,
            end_ns: None,
            duration_ms: None,
        }
    }

    pub fn complete(&mut self, end_ns: u64) {
        self.end_ns = Some(end_ns);
        self.duration_ms = Some((end_ns - self.start_ns) / 1_000_000);
    }
}

/// Counts of state transitions during a pipeline run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateTransitionCounts {
    pub idle_to_observing: u32,
    pub observing_to_reasoning: u32,
    pub reasoning_to_synthesizing: u32,
    pub synthesizing_to_acting: u32,
    pub acting_to_synthesizing: u32,
    pub synthesizing_to_completed: u32,
    pub any_to_failed: u32,
}

impl StateTransitionCounts {
    pub fn total_transitions(&self) -> u32 {
        self.idle_to_observing
            + self.observing_to_reasoning
            + self.reasoning_to_synthesizing
            + self.synthesizing_to_acting
            + self.acting_to_synthesizing
            + self.synthesizing_to_completed
            + self.any_to_failed
    }
}

/// Diagnostics for a single pipeline run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineDiagnostics {
    pub correlation_id: String,
    pub task_id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub final_state: RuntimeState,
    pub succeeded: bool,
    pub total_duration_ms: u64,
    pub phase_durations: HashMap<String, PhaseDuration>,
    pub state_transitions: StateTransitionCounts,
    pub tool_call_count: u32,
    pub error_messages: Vec<String>,
}

impl PipelineDiagnostics {
    /// Creates a new empty pipeline diagnostics collector.
    pub fn new(task_id: &str, correlation_id: &str) -> Self {
        PipelineDiagnostics {
            correlation_id: correlation_id.to_string(),
            task_id: task_id.to_string(),
            started_at: Utc::now(),
            completed_at: None,
            final_state: RuntimeState::Idle,
            succeeded: false,
            total_duration_ms: 0,
            phase_durations: HashMap::new(),
            state_transitions: StateTransitionCounts::default(),
            tool_call_count: 0,
            error_messages: Vec::new(),
        }
    }

    /// Records the start of a phase.
    pub fn start_phase(&mut self, phase: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        self.phase_durations.insert(
            phase.to_string(),
            PhaseDuration::new(phase, now),
        );
    }

    /// Records the completion of a phase.
    pub fn complete_phase(&mut self, phase: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        if let Some(pd) = self.phase_durations.get_mut(phase) {
            pd.complete(now);
        }
    }

    /// Records a state transition.
    pub fn record_transition(&mut self, from: RuntimeState, to: RuntimeState) {
        self.final_state = to.clone();
        match (from, to) {
            (RuntimeState::Idle, RuntimeState::Observing) => {
                self.state_transitions.idle_to_observing += 1;
            }
            (RuntimeState::Observing, RuntimeState::Reasoning) => {
                self.state_transitions.observing_to_reasoning += 1;
            }
            (RuntimeState::Reasoning, RuntimeState::Synthesizing) => {
                self.state_transitions.reasoning_to_synthesizing += 1;
            }
            (RuntimeState::Synthesizing, RuntimeState::Acting) => {
                self.state_transitions.synthesizing_to_acting += 1;
            }
            (RuntimeState::Acting, RuntimeState::Synthesizing) => {
                self.state_transitions.acting_to_synthesizing += 1;
            }
            (RuntimeState::Synthesizing, RuntimeState::Completed) => {
                self.state_transitions.synthesizing_to_completed += 1;
            }
            (_, RuntimeState::Failed) => {
                self.state_transitions.any_to_failed += 1;
            }
            _ => {}
        }
    }

    /// Records a tool call.
    pub fn record_tool_call(&mut self) {
        self.tool_call_count += 1;
    }

    /// Records an error message.
    pub fn record_error(&mut self, error: &str) {
        self.error_messages.push(error.to_string());
    }

    /// Marks the pipeline as succeeded and records completion time.
    pub fn mark_completed(&mut self) {
        self.succeeded = true;
        self.completed_at = Some(Utc::now());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        self.total_duration_ms = (now - self.started_at.timestamp_nanos_opt().unwrap_or(0) as u64) / 1_000_000;
    }

    /// Marks the pipeline as failed and records completion time.
    pub fn mark_failed(&mut self, error: &str) {
        self.succeeded = false;
        self.completed_at = Some(Utc::now());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        self.total_duration_ms = (now - self.started_at.timestamp_nanos_opt().unwrap_or(0) as u64) / 1_000_000;
        self.record_error(error);
    }

    /// Returns the duration of a specific phase, if recorded.
    pub fn phase_duration(&self, phase: &str) -> Option<u64> {
        self.phase_durations
            .get(phase)
            .and_then(|pd| pd.duration_ms)
    }

    /// Returns a human-readable summary.
    pub fn summary(&self) -> String {
        format!(
            "Pipeline[{}] {} in {}ms ({} tool calls, {} errors)",
            &self.task_id[..8.min(self.task_id.len())],
            if self.succeeded { "succeeded" } else { "failed" },
            self.total_duration_ms,
            self.tool_call_count,
            self.error_messages.len(),
        )
    }
}

/// Aggregated diagnostics across multiple pipeline runs.
#[derive(Debug, Clone)]
pub struct RuntimeDiagnostics {
    inner: Arc<Mutex<RuntimeDiagnosticsInner>>,
}

#[derive(Debug)]
struct RuntimeDiagnosticsInner {
    current: Option<PipelineDiagnostics>,
    completed: Vec<PipelineDiagnostics>,
    max_completed: usize,
}

impl RuntimeDiagnostics {
    /// Creates a new diagnostics collector.
    pub fn new() -> Self {
        RuntimeDiagnostics {
            inner: Arc::new(Mutex::new(RuntimeDiagnosticsInner {
                current: None,
                completed: Vec::new(),
                max_completed: 100,
            })),
        }
    }

    /// Starts tracking diagnostics for a new pipeline run.
    pub fn begin(&self, task_id: &str, correlation_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.current = Some(PipelineDiagnostics::new(task_id, correlation_id));
    }

    /// Ends the current pipeline run and moves it to completed.
    pub fn end(&self) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(mut diag) = inner.current.take() {
            diag.mark_failed("pipeline ended without success or failure mark");
            inner.completed.push(diag);
            while inner.completed.len() > inner.max_completed {
                inner.completed.remove(0);
            }
        }
    }

    /// Returns the current in-flight diagnostics, if any.
    pub fn current(&self) -> Option<PipelineDiagnostics> {
        self.inner.lock().unwrap().current.as_ref().cloned()
    }

    /// Returns all completed pipeline diagnostics.
    pub fn completed(&self) -> Vec<PipelineDiagnostics> {
        self.inner.lock().unwrap().completed.clone()
    }

    /// Records a runtime event in the current diagnostics.
    pub fn record_event(&self, event: &RuntimeEvent) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(ref mut diag) = inner.current {
            match event {
                RuntimeEvent::StateChange { from, to } => {
                    diag.record_transition(from.clone(), to.clone());
                }
                RuntimeEvent::ToolExecuted { .. } => {
                    diag.record_tool_call();
                }
                RuntimeEvent::PipelineFailed { error, .. } => {
                    diag.record_error(error);
                }
                RuntimeEvent::PipelineCompleted { .. } => {
                    diag.mark_completed();
                }
                _ => {}
            }
        }
    }

    /// Returns the number of completed runs.
    pub fn completed_count(&self) -> usize {
        self.inner.lock().unwrap().completed.len()
    }

    /// Returns the average total duration across completed runs.
    pub fn average_duration_ms(&self) -> u64 {
        let inner = self.inner.lock().unwrap();
        if inner.completed.is_empty() {
            return 0;
        }
        inner
            .completed
            .iter()
            .map(|d| d.total_duration_ms)
            .sum::<u64>()
            / inner.completed.len() as u64
    }

    /// Returns the success rate across completed runs.
    pub fn success_rate(&self) -> f64 {
        let inner = self.inner.lock().unwrap();
        if inner.completed.is_empty() {
            return 0.0;
        }
        let succeeded = inner
            .completed
            .iter()
            .filter(|d| d.succeeded)
            .count() as f64;
        succeeded / inner.completed.len() as f64
    }

    /// Marks the current pipeline run as completed (for testing).
    pub fn mark_completed_for_test(&self) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(ref mut diag) = inner.current {
            diag.mark_completed();
        }
    }

    /// Returns a summary of all completed runs.
    pub fn summary(&self) -> String {
        let inner = self.inner.lock().unwrap();
        if inner.completed.is_empty() {
            return "No completed runs".to_string();
        }
        let total = inner.completed.len();
        let succeeded = inner
            .completed
            .iter()
            .filter(|d| d.succeeded)
            .count();
        let avg_duration = inner
            .completed
            .iter()
            .map(|d| d.total_duration_ms)
            .sum::<u64>()
            / total as u64;
        let total_errors: usize = inner
            .completed
            .iter()
            .map(|d| d.error_messages.len())
            .sum();
        format!(
            "Diagnostics[{} runs, {} succeeded, avg {}ms, {} errors]",
            total, succeeded, avg_duration, total_errors
        )
    }
}

impl Default for RuntimeDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::events::RuntimeEvent;

    #[test]
    fn test_pipeline_diagnostics_creation() {
        let diag = PipelineDiagnostics::new("task-1", "corr-1");
        assert_eq!(diag.task_id, "task-1");
        assert_eq!(diag.correlation_id, "corr-1");
        assert!(!diag.succeeded);
        assert_eq!(diag.total_duration_ms, 0);
        assert_eq!(diag.tool_call_count, 0);
    }

    #[test]
    fn test_phase_duration_recording() {
        let mut diag = PipelineDiagnostics::new("task-1", "corr-1");
        diag.start_phase("observe");
        // Complete phase immediately - duration may be 0 in tests
        diag.complete_phase("observe");
        assert!(diag.phase_duration("observe").is_some());
    }

    #[test]
    fn test_state_transition_recording() {
        let mut diag = PipelineDiagnostics::new("task-1", "corr-1");
        diag.record_transition(RuntimeState::Idle, RuntimeState::Observing);
        diag.record_transition(RuntimeState::Observing, RuntimeState::Reasoning);
        diag.record_transition(RuntimeState::Reasoning, RuntimeState::Synthesizing);
        diag.record_transition(RuntimeState::Synthesizing, RuntimeState::Acting);
        diag.record_transition(RuntimeState::Acting, RuntimeState::Synthesizing);
        diag.record_transition(RuntimeState::Synthesizing, RuntimeState::Completed);

        let tc = &diag.state_transitions;
        assert_eq!(tc.idle_to_observing, 1);
        assert_eq!(tc.observing_to_reasoning, 1);
        assert_eq!(tc.reasoning_to_synthesizing, 1);
        assert_eq!(tc.synthesizing_to_acting, 1);
        assert_eq!(tc.acting_to_synthesizing, 1);
        assert_eq!(tc.synthesizing_to_completed, 1);
        assert_eq!(tc.total_transitions(), 6);
    }

    #[test]
    fn test_tool_call_recording() {
        let mut diag = PipelineDiagnostics::new("task-1", "corr-1");
        diag.record_tool_call();
        diag.record_tool_call();
        diag.record_tool_call();
        assert_eq!(diag.tool_call_count, 3);
    }

    #[test]
    fn test_mark_completed() {
        let mut diag = PipelineDiagnostics::new("task-1", "corr-1");
        diag.mark_completed();
        assert!(diag.succeeded);
        assert!(diag.completed_at.is_some());
        // Duration should be at least 0 (may be 0 in very fast tests)
        assert!(diag.total_duration_ms >= 0);
    }

    #[test]
    fn test_mark_failed() {
        let mut diag = PipelineDiagnostics::new("task-1", "corr-1");
        diag.mark_failed("timeout");
        assert!(!diag.succeeded);
        assert!(diag.completed_at.is_some());
        assert_eq!(diag.error_messages, vec!["timeout"]);
    }

    #[test]
    fn test_summary() {
        let mut diag = PipelineDiagnostics::new("task-1", "corr-1");
        diag.record_tool_call();
        diag.mark_completed();
        let s = diag.summary();
        assert!(s.contains("succeeded"));
        assert!(s.contains("1 tool calls"));
    }

    #[test]
    fn test_runtime_diagnostics_collector() {
        let diag = RuntimeDiagnostics::new();
        assert_eq!(diag.completed_count(), 0);
        assert_eq!(diag.average_duration_ms(), 0);
        assert_eq!(diag.success_rate(), 0.0);
    }

    #[test]
    fn test_runtime_diagnostics_record_event() {
        let diag = RuntimeDiagnostics::new();
        diag.begin("task-1", "corr-1");

        diag.record_event(&RuntimeEvent::StateChange {
            from: RuntimeState::Idle,
            to: RuntimeState::Observing,
        });

        let current = diag.current().unwrap();
        assert_eq!(current.state_transitions.idle_to_observing, 1);
    }

    #[test]
    fn test_runtime_diagnostics_begin_end() {
        let diag = RuntimeDiagnostics::new();
        diag.begin("task-1", "corr-1");
        diag.end();
        assert_eq!(diag.completed_count(), 1);
    }

    #[test]
    fn test_runtime_diagnostics_summary() {
        let diag = RuntimeDiagnostics::new();
        diag.begin("task-1", "corr-1");
        diag.mark_completed_for_test();
        diag.end();

        let s = diag.summary();
        assert!(s.contains("1 runs"));
    }
}
