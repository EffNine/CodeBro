#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Runtime state machine for the CodeBro ReAct loop.
//!
//! The runtime progresses through discrete states:
//! Idle → Observing → Reasoning → Synthesizing → (Acting → Synthesizing)* → Completed/Failed
//!
//! This replaces the previous monolithic `run_chat_pipeline` function with
//! explicit, testable state transitions.

#[derive(Debug, Clone, PartialEq, Eq, Copy, Hash, serde::Serialize, serde::Deserialize)]
pub enum RuntimeState {
    /// Waiting for user input.
    Idle,
    /// Gathering ground truth via the tool pipeline.
    Observing,
    /// Coordinator/subagents analyzing the task.
    Reasoning,
    /// Streaming a response from the LLM provider.
    Synthesizing,
    /// Executing tool calls returned by the LLM.
    Acting,
    /// Task completed successfully.
    Completed,
    /// Task failed.
    Failed,
    /// Task was cancelled by the user.
    Cancelled,
}

impl RuntimeState {
    /// Returns the set of valid next states from this state.
    pub fn valid_transitions(&self) -> &'static [RuntimeState] {
        match self {
            RuntimeState::Idle => &[RuntimeState::Observing],
            RuntimeState::Observing => &[RuntimeState::Reasoning, RuntimeState::Failed],
            RuntimeState::Reasoning => &[RuntimeState::Synthesizing, RuntimeState::Failed],
            RuntimeState::Synthesizing => &[
                RuntimeState::Acting,
                RuntimeState::Completed,
                RuntimeState::Failed,
                RuntimeState::Cancelled,
            ],
            RuntimeState::Acting => &[
                RuntimeState::Synthesizing,
                RuntimeState::Failed,
                RuntimeState::Cancelled,
            ],
            RuntimeState::Completed | RuntimeState::Failed | RuntimeState::Cancelled => &[],
        }
    }

    /// Attempts to transition to the next state.
    /// Returns `Err` if the transition is invalid.
    pub fn try_transition(self, next: RuntimeState) -> Result<RuntimeState, RuntimeError> {
        if self.valid_transitions().contains(&next) {
            Ok(next)
        } else {
            Err(RuntimeError {
                from: self,
                to: next,
            })
        }
    }

    /// Returns `true` if this is a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RuntimeState::Completed | RuntimeState::Failed | RuntimeState::Cancelled
        )
    }

    /// Returns `true` if the runtime is currently active (not idle/terminal).
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            RuntimeState::Observing
                | RuntimeState::Reasoning
                | RuntimeState::Synthesizing
                | RuntimeState::Acting
        )
    }
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self::Idle
    }
}

/// Error returned when an invalid state transition is attempted.
#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub from: RuntimeState,
    pub to: RuntimeState,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid transition: {:?} -> {:?}", self.from, self.to)
    }
}

impl std::error::Error for RuntimeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_idle_transitions_to_observing() {
        let state = RuntimeState::Idle;
        assert!(state.try_transition(RuntimeState::Observing).is_ok());
    }

    #[test]
    fn test_observing_transitions_to_reasoning() {
        let state = RuntimeState::Observing;
        assert!(state.try_transition(RuntimeState::Reasoning).is_ok());
    }

    #[test]
    fn test_reasoning_transitions_to_synthesizing() {
        let state = RuntimeState::Reasoning;
        assert!(state.try_transition(RuntimeState::Synthesizing).is_ok());
    }

    #[test]
    fn test_synthesizing_transitions_to_acting_or_completed() {
        let state = RuntimeState::Synthesizing;
        assert!(state.try_transition(RuntimeState::Acting).is_ok());
        assert!(state.try_transition(RuntimeState::Completed).is_ok());
    }

    #[test]
    fn test_acting_transitions_back_to_synthesizing() {
        let state = RuntimeState::Acting;
        assert!(state.try_transition(RuntimeState::Synthesizing).is_ok());
    }

    #[test]
    fn test_completed_is_terminal() {
        assert!(RuntimeState::Completed.is_terminal());
        assert!(RuntimeState::Failed.is_terminal());
        assert!(RuntimeState::Cancelled.is_terminal());
        assert!(!RuntimeState::Idle.is_terminal());
        assert!(!RuntimeState::Observing.is_terminal());
    }

    #[test]
    fn test_invalid_transition_rejected() {
        let state = RuntimeState::Idle;
        assert!(state.try_transition(RuntimeState::Completed).is_err());
        assert!(state.try_transition(RuntimeState::Reasoning).is_err());

        let state = RuntimeState::Completed;
        assert!(state.try_transition(RuntimeState::Observing).is_err());
    }

    #[test]
    fn test_full_pipeline_sequence() {
        let mut state = RuntimeState::Idle;
        state = state.try_transition(RuntimeState::Observing).unwrap();
        state = state.try_transition(RuntimeState::Reasoning).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Acting).unwrap();
        state = state.try_transition(RuntimeState::Synthesizing).unwrap();
        state = state.try_transition(RuntimeState::Completed).unwrap();
        assert!(state.is_terminal());
    }

    #[test]
    fn test_synthesizing_transitions_to_cancelled() {
        let state = RuntimeState::Synthesizing;
        assert!(state.try_transition(RuntimeState::Cancelled).is_ok());
    }

    #[test]
    fn test_acting_transitions_to_cancelled() {
        let state = RuntimeState::Acting;
        assert!(state.try_transition(RuntimeState::Cancelled).is_ok());
    }

    #[test]
    fn test_cancelled_is_terminal_and_invalid_from_terminal() {
        assert!(RuntimeState::Cancelled.is_terminal());
        let state = RuntimeState::Cancelled;
        assert!(state.try_transition(RuntimeState::Observing).is_err());
        assert!(state.try_transition(RuntimeState::Completed).is_err());
    }

    #[test]
    fn test_is_active() {
        assert!(!RuntimeState::Idle.is_active());
        assert!(RuntimeState::Observing.is_active());
        assert!(RuntimeState::Reasoning.is_active());
        assert!(RuntimeState::Synthesizing.is_active());
        assert!(RuntimeState::Acting.is_active());
        assert!(!RuntimeState::Completed.is_active());
        assert!(!RuntimeState::Failed.is_active());
        assert!(!RuntimeState::Cancelled.is_active());
    }
}
