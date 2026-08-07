#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Runtime lifecycle management for CodeBro.
//!
//! `RuntimeLifecycle` manages the observable lifecycle of a runtime
//! session from creation through shutdown. It is separate from
//! `RuntimeState` (which tracks pipeline phase) and `RuntimeState`
//! (which lives in `state.rs`) — this type tracks the *hosting* lifecycle.
//!
//! Lifecycle states:
//!
//! ```text
//! Created → Running → (Paused) → Running → Stopping → Stopped
//!                          ↘ ShuttingDown → Stopped
//! ```
//!
//! Transitions are validated; invalid transitions return `RuntimeError`.

use std::time::Instant;

use chrono::{DateTime, Utc};

use super::state::RuntimeState;

/// The host-level lifecycle state of the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Copy, serde::Serialize, serde::Deserialize)]
pub enum RuntimeLifecycleState {
    /// Runtime object created but not yet started.
    Created,
    /// Runtime is actively processing tasks.
    Running,
    /// Runtime is temporarily paused (e.g., user suspended).
    Paused,
    /// Runtime is in the process of shutting down.
    Stopping,
    /// Runtime has been stopped.
    Stopped,
}

impl RuntimeLifecycleState {
    /// Returns whether the runtime is alive (not Stopped).
    pub fn is_alive(&self) -> bool {
        !matches!(self, RuntimeLifecycleState::Stopped)
    }

    /// Returns whether the runtime is currently active.
    pub fn is_active(&self) -> bool {
        matches!(self, RuntimeLifecycleState::Running)
    }

    /// Returns a human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            RuntimeLifecycleState::Created => "created",
            RuntimeLifecycleState::Running => "running",
            RuntimeLifecycleState::Paused => "paused",
            RuntimeLifecycleState::Stopping => "stopping",
            RuntimeLifecycleState::Stopped => "stopped",
        }
    }
}

impl Default for RuntimeLifecycleState {
    fn default() -> Self {
        Self::Created
    }
}

/// Valid next states from a given lifecycle state.
fn valid_transitions_from(state: &RuntimeLifecycleState) -> &'static [RuntimeLifecycleState] {
    match state {
        RuntimeLifecycleState::Created => &[RuntimeLifecycleState::Running],
        RuntimeLifecycleState::Running => &[
            RuntimeLifecycleState::Paused,
            RuntimeLifecycleState::Stopping,
        ],
        RuntimeLifecycleState::Paused => &[
            RuntimeLifecycleState::Running,
            RuntimeLifecycleState::Stopping,
        ],
        RuntimeLifecycleState::Stopping => &[RuntimeLifecycleState::Stopped],
        RuntimeLifecycleState::Stopped => &[],
    }
}

/// Tracks the lifecycle of a runtime instance.
///
/// This is separate from `RuntimeState` (pipeline phase). A single
/// `RuntimeLifecycle` may host many `RuntimeState` cycles.
#[derive(Debug, Clone)]
pub struct RuntimeLifecycle {
    state: RuntimeLifecycleState,
    started_at: Option<DateTime<Utc>>,
    stopped_at: Option<DateTime<Utc>>,
    pause_started_at: Option<Instant>,
    total_running_ms: u64,
    task_count: u32,
}

impl RuntimeLifecycle {
    /// Creates a new lifecycle in the `Created` state.
    pub fn new() -> Self {
        RuntimeLifecycle {
            state: RuntimeLifecycleState::Created,
            started_at: None,
            stopped_at: None,
            pause_started_at: None,
            total_running_ms: 0,
            task_count: 0,
        }
    }

    /// Returns the current lifecycle state.
    pub fn state(&self) -> RuntimeLifecycleState {
        self.state
    }

    /// Returns whether the lifecycle is in the given state.
    pub fn is(&self, s: RuntimeLifecycleState) -> bool {
        self.state == s
    }

    /// Attempts to transition to the given state.
    ///
    /// Returns `Err` if the transition is invalid.
    pub fn try_transition(
        &mut self,
        next: RuntimeLifecycleState,
    ) -> Result<RuntimeLifecycleState, RuntimeError> {
        let valid = valid_transitions_from(&self.state);
        if valid.contains(&next) {
            let previous = self.state;
            self.state = next;

            match next {
                RuntimeLifecycleState::Running => {
                    if self.pause_started_at.is_some() {
                        // Account for pause duration
                        if let Some(start) = self.pause_started_at {
                            let pause_ms = start.elapsed().as_millis() as u64;
                            self.total_running_ms = self
                                .total_running_ms
                                .saturating_add(pause_ms);
                        }
                        self.pause_started_at = None;
                    }
                    if self.started_at.is_none() {
                        self.started_at = Some(Utc::now());
                    }
                }
                RuntimeLifecycleState::Paused => {
                    self.pause_started_at = Some(Instant::now());
                }
                RuntimeLifecycleState::Stopping => {
                    // No extra bookkeeping
                }
                RuntimeLifecycleState::Stopped => {
                    self.stopped_at = Some(Utc::now());
                    if let Some(pause_start) = self.pause_started_at {
                        let pause_ms = pause_start.elapsed().as_millis() as u64;
                        self.total_running_ms = self
                            .total_running_ms
                            .saturating_add(pause_ms);
                        self.pause_started_at = None;
                    }
                }
                _ => {}
            }

            Ok(next)
        } else {
            Err(RuntimeError {
                from: self.state,
                to: next,
            })
        }
    }

    /// Transitions to `Running`. Convenience wrapper.
    pub fn start(&mut self) -> Result<(), RuntimeError> {
        self.try_transition(RuntimeLifecycleState::Running).map(|_| ())
    }

    /// Transitions to `Paused`. Convenience wrapper.
    pub fn pause(&mut self) -> Result<(), RuntimeError> {
        self.try_transition(RuntimeLifecycleState::Paused).map(|_| ())
    }

    /// Transitions to `Running` from `Paused`. Convenience wrapper.
    pub fn resume(&mut self) -> Result<(), RuntimeError> {
        self.try_transition(RuntimeLifecycleState::Running).map(|_| ())
    }

    /// Transitions to `Stopping`. Convenience wrapper.
    pub fn stop(&mut self) -> Result<(), RuntimeError> {
        self.try_transition(RuntimeLifecycleState::Stopping).map(|_| ())
    }

    /// Transitions to `Stopped`. Convenience wrapper.
    pub fn shutdown(&mut self) -> Result<(), RuntimeError> {
        self.try_transition(RuntimeLifecycleState::Stopped).map(|_| ())
    }

    /// Returns the time at which the runtime entered `Running` for the
    /// first time, if any.
    pub fn started_at(&self) -> Option<DateTime<Utc>> {
        self.started_at
    }

    /// Returns the time at which the runtime entered `Stopped`.
    pub fn stopped_at(&self) -> Option<DateTime<Utc>> {
        self.stopped_at
    }

    /// Returns the total accumulated running time in milliseconds.
    pub fn total_running_ms(&self) -> u64 {
        self.total_running_ms
    }

    /// Increments the task counter.
    pub fn record_task(&mut self) {
        self.task_count += 1;
    }

    /// Returns the number of tasks processed.
    pub fn task_count(&self) -> u32 {
        self.task_count
    }

    /// Returns the current uptime in milliseconds (time since first start,
    /// excluding pauses).
    pub fn uptime_ms(&self) -> u64 {
        if let Some(start) = self.started_at {
            let elapsed = Utc::now()
                .signed_duration_since(start)
                .to_std()
                .unwrap_or_default()
                .as_millis() as u64;
            elapsed.saturating_sub(self.total_running_ms)
        } else {
            0
        }
    }

    /// Returns a summary string.
    pub fn summary(&self) -> String {
        format!(
            "lifecycle={} tasks={} uptime_ms={}",
            self.state.label(),
            self.task_count,
            self.uptime_ms(),
        )
    }
}

impl Default for RuntimeLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

/// Error returned when an invalid lifecycle transition is attempted.
#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub from: RuntimeLifecycleState,
    pub to: RuntimeLifecycleState,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Invalid lifecycle transition: {} -> {}",
            self.from.label(),
            self.to.label(),
        )
    }
}

impl std::error::Error for RuntimeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state_is_created() {
        let lc = RuntimeLifecycle::new();
        assert_eq!(lc.state(), RuntimeLifecycleState::Created);
        assert!(lc.state().is_alive()); // Created is alive but not active
    }

    #[test]
    fn test_start_from_created() {
        let mut lc = RuntimeLifecycle::new();
        assert!(lc.start().is_ok());
        assert_eq!(lc.state(), RuntimeLifecycleState::Running);
        assert!(lc.started_at().is_some());
    }

    #[test]
    fn test_pause_and_resume() {
        let mut lc = RuntimeLifecycle::new();
        lc.start().unwrap();
        assert!(lc.pause().is_ok());
        assert_eq!(lc.state(), RuntimeLifecycleState::Paused);
        assert!(lc.resume().is_ok());
        assert_eq!(lc.state(), RuntimeLifecycleState::Running);
    }

    #[test]
    fn test_stop_and_shutdown() {
        let mut lc = RuntimeLifecycle::new();
        lc.start().unwrap();
        assert!(lc.stop().is_ok());
        assert_eq!(lc.state(), RuntimeLifecycleState::Stopping);
        assert!(lc.shutdown().is_ok());
        assert_eq!(lc.state(), RuntimeLifecycleState::Stopped);
        assert!(!lc.state().is_alive());
    }

    #[test]
    fn test_invalid_transitions_rejected() {
        let mut lc = RuntimeLifecycle::new();
        // Cannot stop from Created
        assert!(lc.shutdown().is_err());
        // Cannot pause from Created
        assert!(lc.pause().is_err());

        lc.start().unwrap();
        // Cannot go back to Created
        assert!(lc.try_transition(RuntimeLifecycleState::Created).is_err());
        // Cannot go directly to Stopped (must go through Stopping)
        assert!(lc.shutdown().is_err());
    }

    #[test]
    fn test_task_counting() {
        let mut lc = RuntimeLifecycle::new();
        lc.start().unwrap();
        assert_eq!(lc.task_count(), 0);
        lc.record_task();
        lc.record_task();
        lc.record_task();
        assert_eq!(lc.task_count(), 3);
    }

    #[test]
    fn test_summary() {
        let mut lc = RuntimeLifecycle::new();
        lc.start().unwrap();
        lc.record_task();
        let s = lc.summary();
        assert!(s.contains("running"));
        assert!(s.contains("tasks=1"));
    }

    #[test]
    fn test_is_active() {
        let mut lc = RuntimeLifecycle::new();
        assert!(!lc.state().is_active());
        lc.start().unwrap();
        assert!(lc.state().is_active());
        lc.pause().unwrap();
        assert!(!lc.state().is_active());
    }
}
