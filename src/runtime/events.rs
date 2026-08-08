#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Runtime events for CodeBro.
//!
//! `RuntimeEvent` is the event type emitted by the runtime pipeline to
//! notify the TUI and other observers of pipeline progress. It is
//! separate from `AgentEvent` (which is emitted by the agent layer) and
//! `AppEvent` (which is the top-level TUI event type).
//!
//! The runtime pipeline emits `RuntimeEvent` which is converted to
//! `AppEvent::RuntimeEvent(...)` by the TUI layer.

use serde::{Deserialize, Serialize};

use super::state::RuntimeState;

/// Events emitted by the runtime pipeline.
///
/// These events are observed by the TUI, session tracker, and diagnostics
/// subsystems. They do not carry large payloads — long strings are
/// summarized.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RuntimeEvent {
    /// The pipeline has started a new task.
    PipelineStarted {
        task_id: String,
        correlation_id: String,
        user_request_summary: String,
    },

    /// The pipeline is transitioning to a new state.
    StateChange {
        from: RuntimeState,
        to: RuntimeState,
    },

    /// The observe phase completed with tool context.
    ObserveComplete {
        tool_context_summary: String,
        duration_ms: u64,
    },

    /// The reason phase completed with a report.
    ReasonComplete {
        report_summary: String,
        duration_ms: u64,
    },

    /// A stream chunk was received from the provider.
    StreamChunk { chunk: String },

    /// Synthesis completed with a final response.
    SynthesizeComplete {
        response_summary: String,
        duration_ms: u64,
        tool_calls_found: u32,
    },

    /// A tool call was executed during the act phase.
    ToolExecuted {
        tool_name: String,
        args_summary: String,
        result_summary: String,
        success: bool,
        duration_ms: u64,
    },

    /// The act phase completed (either more synthesis or done).
    ActComplete {
        loop_count: u32,
        total_tool_calls: u32,
    },

    /// The pipeline completed successfully.
    PipelineCompleted {
        duration_ms: u64,
        tool_calls_total: u32,
        response_length: usize,
    },

    /// The pipeline failed with an error.
    PipelineFailed {
        error: String,
        duration_ms: u64,
        state_at_failure: RuntimeState,
    },

    /// A lifecycle event (start, pause, resume, stop).
    LifecycleEvent {
        from: super::lifecycle::RuntimeLifecycleState,
        to: super::lifecycle::RuntimeLifecycleState,
    },

    /// Diagnostic data collected during the pipeline run.
    DiagnosticsCollected {
        correlation_id: String,
        failure_count: usize,
        recovery_count: usize,
    },
}

impl RuntimeEvent {
    /// Returns a short summary string suitable for display in the TUI.
    pub fn summary(&self) -> String {
        match self {
            RuntimeEvent::PipelineStarted {
                task_id,
                user_request_summary,
                ..
            } => format!(
                "Pipeline started: {} — {}",
                &task_id[..task_id.len().min(8)],
                user_request_summary
            ),
            RuntimeEvent::StateChange { from, to } => {
                format!("{:?} → {:?}", from, to)
            }
            RuntimeEvent::ObserveComplete { duration_ms, .. } => {
                format!("Observe complete ({}ms)", duration_ms)
            }
            RuntimeEvent::ReasonComplete { duration_ms, .. } => {
                format!("Reason complete ({}ms)", duration_ms)
            }
            RuntimeEvent::StreamChunk { chunk } => {
                let truncated = if chunk.len() > 50 {
                    format!("{}...", &chunk[..50])
                } else {
                    chunk.clone()
                };
                format!("Stream: {}", truncated)
            }
            RuntimeEvent::SynthesizeComplete {
                response_summary,
                duration_ms,
                tool_calls_found,
                ..
            } => {
                let preview = if response_summary.len() > 50 {
                    &response_summary[..50]
                } else {
                    response_summary.as_str()
                };
                format!(
                    "Synthesize complete ({}ms, {} tool calls) — {}",
                    duration_ms, tool_calls_found, preview
                )
            }
            RuntimeEvent::ToolExecuted {
                tool_name,
                success,
                duration_ms,
                ..
            } => format!(
                "Tool {:?} {} ({}ms)",
                tool_name,
                if *success { "ok" } else { "FAIL" },
                duration_ms
            ),
            RuntimeEvent::ActComplete {
                loop_count,
                total_tool_calls,
            } => format!(
                "Act complete: {} loops, {} tool calls",
                loop_count, total_tool_calls
            ),
            RuntimeEvent::PipelineCompleted {
                duration_ms,
                tool_calls_total,
                ..
            } => format!(
                "Pipeline completed in {}ms ({} tool calls)",
                duration_ms, tool_calls_total
            ),
            RuntimeEvent::PipelineFailed {
                error, duration_ms, ..
            } => {
                format!("Pipeline failed after {}ms: {}", duration_ms, error)
            }
            RuntimeEvent::LifecycleEvent { from, to } => {
                format!("{:?} → {:?}", from, to)
            }
            RuntimeEvent::DiagnosticsCollected {
                failure_count,
                recovery_count,
                ..
            } => format!(
                "Diagnostics: {} failures, {} recoveries",
                failure_count, recovery_count
            ),
        }
    }

    /// Returns `true` if this event represents a terminal pipeline state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RuntimeEvent::PipelineCompleted { .. } | RuntimeEvent::PipelineFailed { .. }
        )
    }

    /// Returns the runtime state associated with this event, if any.
    pub fn associated_state(&self) -> Option<RuntimeState> {
        match self {
            RuntimeEvent::StateChange { to, .. } => Some(to.clone()),
            RuntimeEvent::PipelineFailed {
                state_at_failure, ..
            } => Some(state_at_failure.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::lifecycle::RuntimeLifecycleState;
    use super::super::state::RuntimeState;
    use super::*;

    #[test]
    fn test_pipeline_started_summary() {
        let event = RuntimeEvent::PipelineStarted {
            task_id: "abc123".to_string(),
            correlation_id: "corr1".to_string(),
            user_request_summary: "read main.rs".to_string(),
        };
        let s = event.summary();
        assert!(s.contains("abc123"));
        assert!(s.contains("read main.rs"));
    }

    #[test]
    fn test_state_change_summary() {
        let event = RuntimeEvent::StateChange {
            from: RuntimeState::Idle,
            to: RuntimeState::Observing,
        };
        let s = event.summary();
        assert!(s.contains("Idle"));
        assert!(s.contains("Observing"));
    }

    #[test]
    fn test_is_terminal() {
        assert!(RuntimeEvent::PipelineCompleted {
            duration_ms: 100,
            tool_calls_total: 2,
            response_length: 50,
        }
        .is_terminal());
        assert!(RuntimeEvent::PipelineFailed {
            error: "timeout".to_string(),
            duration_ms: 100,
            state_at_failure: RuntimeState::Synthesizing,
        }
        .is_terminal());
        assert!(!RuntimeEvent::StateChange {
            from: RuntimeState::Idle,
            to: RuntimeState::Observing,
        }
        .is_terminal());
    }

    #[test]
    fn test_associated_state() {
        let event = RuntimeEvent::StateChange {
            from: RuntimeState::Idle,
            to: RuntimeState::Acting,
        };
        assert_eq!(event.associated_state(), Some(RuntimeState::Acting));

        let event = RuntimeEvent::PipelineStarted {
            task_id: "t".to_string(),
            correlation_id: "c".to_string(),
            user_request_summary: "s".to_string(),
        };
        assert_eq!(event.associated_state(), None);
    }

    #[test]
    fn test_lifecycle_event_summary() {
        let event = RuntimeEvent::LifecycleEvent {
            from: RuntimeLifecycleState::Created,
            to: RuntimeLifecycleState::Running,
        };
        let s = event.summary();
        assert!(s.contains("Created"));
        assert!(s.contains("Running"));
    }
}
