//! Tool Lifecycle Management
//!
//! Defines the state machine for tool registration, enablement, and deprecation.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Lifecycle states for a registered tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolLifecycleState {
    /// Tool has been defined but not yet registered in any registry.
    Unregistered,
    /// Tool is registered but not yet enabled for use.
    Registered,
    /// Tool is active and can be dispatched.
    Enabled,
    /// Tool is temporarily unavailable (e.g., disabled by policy).
    Disabled,
    /// Tool is being phased out; still functional but warns users.
    Deprecating,
    /// Tool has been removed and is no longer available.
    Removed,
}

impl fmt::Display for ToolLifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolLifecycleState::Unregistered => write!(f, "unregistered"),
            ToolLifecycleState::Registered => write!(f, "registered"),
            ToolLifecycleState::Enabled => write!(f, "enabled"),
            ToolLifecycleState::Disabled => write!(f, "disabled"),
            ToolLifecycleState::Deprecating => write!(f, "deprecating"),
            ToolLifecycleState::Removed => write!(f, "removed"),
        }
    }
}

/// Valid state transitions for tools.
#[rustfmt::skip]
const VALID_TRANSITIONS: &[(ToolLifecycleState, ToolLifecycleState)] = &[
    (ToolLifecycleState::Unregistered, ToolLifecycleState::Registered),
    (ToolLifecycleState::Registered, ToolLifecycleState::Enabled),
    (ToolLifecycleState::Registered, ToolLifecycleState::Disabled),
    (ToolLifecycleState::Enabled,   ToolLifecycleState::Disabled),
    (ToolLifecycleState::Disabled,  ToolLifecycleState::Enabled),
    (ToolLifecycleState::Enabled,   ToolLifecycleState::Deprecating),
    (ToolLifecycleState::Registered,ToolLifecycleState::Deprecating),
    (ToolLifecycleState::Deprecating, ToolLifecycleState::Removed),
];

impl ToolLifecycleState {
    /// Check if a transition from this state to `next` is valid.
    pub fn can_transition_to(&self, next: &ToolLifecycleState) -> bool {
        VALID_TRANSITIONS
            .iter()
            .any(|(from, to)| from == self && to == next)
    }

    /// Perform a state transition, returning the new state or an error.
    pub fn transition_to(
        self,
        next: ToolLifecycleState,
    ) -> Result<ToolLifecycleState, LifecycleError> {
        if self.can_transition_to(&next) {
            Ok(next)
        } else {
            Err(LifecycleError {
                from: self,
                to: next,
            })
        }
    }

    /// Check if the tool is usable (enabled or deprecating).
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            ToolLifecycleState::Enabled | ToolLifecycleState::Deprecating
        )
    }

    /// Check if the tool is permanently gone.
    pub fn is_terminal(&self) -> bool {
        matches!(self, ToolLifecycleState::Removed)
    }

    /// Check if the tool requires a warning to be shown.
    pub fn requires_warning(&self) -> bool {
        matches!(self, ToolLifecycleState::Deprecating)
    }
}

/// Error returned when an invalid lifecycle transition is attempted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleError {
    pub from: ToolLifecycleState,
    pub to: ToolLifecycleState,
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Invalid lifecycle transition: {} -> {}",
            self.from, self.to
        )
    }
}

impl std::error::Error for LifecycleError {}

/// Tracks the lifecycle state for a single tool.
#[derive(Debug, Clone)]
pub struct ToolLifecycle {
    state: ToolLifecycleState,
    history: Vec<(ToolLifecycleState, String /* timestamp */)>,
}

impl ToolLifecycle {
    /// Create a new lifecycle in the Unregistered state.
    pub fn new() -> Self {
        ToolLifecycle {
            state: ToolLifecycleState::Unregistered,
            history: Vec::new(),
        }
    }

    /// Get the current state.
    pub fn state(&self) -> ToolLifecycleState {
        self.state
    }

    /// Get the transition history.
    pub fn history(&self) -> &[(ToolLifecycleState, String)] {
        &self.history
    }

    /// Attempt to transition to a new state.
    pub fn transition(
        &mut self,
        next: ToolLifecycleState,
    ) -> Result<ToolLifecycleState, LifecycleError> {
        let old = self.state;
        let new_state = old.transition_to(next)?;
        self.state = new_state;
        self.history
            .push((new_state, chrono::Utc::now().to_rfc3339()));
        Ok(new_state)
    }

    /// Register the tool (Unregistered -> Registered).
    pub fn register(&mut self) -> Result<(), LifecycleError> {
        let _ = self.transition(ToolLifecycleState::Registered)?;
        Ok(())
    }

    /// Enable the tool (Registered -> Enabled).
    pub fn enable(&mut self) -> Result<(), LifecycleError> {
        let _ = self.transition(ToolLifecycleState::Enabled)?;
        Ok(())
    }

    /// Disable the tool.
    pub fn disable(&mut self) -> Result<(), LifecycleError> {
        let _ = self.transition(ToolLifecycleState::Disabled)?;
        Ok(())
    }

    /// Deprecate the tool.
    pub fn deprecate(&mut self) -> Result<(), LifecycleError> {
        let _ = self.transition(ToolLifecycleState::Deprecating)?;
        Ok(())
    }

    /// Remove the tool.
    pub fn remove(&mut self) -> Result<(), LifecycleError> {
        let _ = self.transition(ToolLifecycleState::Removed)?;
        Ok(())
    }
}

impl Default for ToolLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

/// Manager for tool lifecycles across all registered tools.
#[derive(Debug, Default)]
pub struct LifecycleManager {
    lifecycles: std::collections::HashMap<String, ToolLifecycle>,
}

impl LifecycleManager {
    /// Create a new lifecycle manager.
    pub fn new() -> Self {
        LifecycleManager {
            lifecycles: std::collections::HashMap::new(),
        }
    }

    /// Register a new tool lifecycle.
    pub fn register(&mut self, tool_name: &str) -> Result<(), LifecycleError> {
        let mut lifecycle = ToolLifecycle::new();
        lifecycle.register()?;
        self.lifecycles.insert(tool_name.to_string(), lifecycle);
        Ok(())
    }

    /// Enable a tool.
    pub fn enable(&mut self, tool_name: &str) -> Result<(), LifecycleError> {
        match self.lifecycles.get_mut(tool_name) {
            Some(lc) => lc.enable(),
            None => Err(LifecycleError {
                from: ToolLifecycleState::Unregistered,
                to: ToolLifecycleState::Enabled,
            }),
        }
    }

    /// Disable a tool.
    pub fn disable(&mut self, tool_name: &str) -> Result<(), LifecycleError> {
        match self.lifecycles.get_mut(tool_name) {
            Some(lc) => lc.disable(),
            None => Err(LifecycleError {
                from: ToolLifecycleState::Unregistered,
                to: ToolLifecycleState::Disabled,
            }),
        }
    }

    /// Deprecate a tool.
    pub fn deprecate(&mut self, tool_name: &str) -> Result<(), LifecycleError> {
        match self.lifecycles.get_mut(tool_name) {
            Some(lc) => lc.deprecate(),
            None => Err(LifecycleError {
                from: ToolLifecycleState::Unregistered,
                to: ToolLifecycleState::Deprecating,
            }),
        }
    }

    /// Get the lifecycle state of a tool.
    pub fn state(&self, tool_name: &str) -> Option<ToolLifecycleState> {
        self.lifecycles.get(tool_name).map(|lc| lc.state())
    }

    /// Check if a tool is active (enabled or deprecating).
    pub fn is_active(&self, tool_name: &str) -> bool {
        self.state(tool_name)
            .map(|s| s.is_active())
            .unwrap_or(false)
    }

    /// Get all tool names and their states.
    pub fn all_states(&self) -> Vec<(&str, ToolLifecycleState)> {
        self.lifecycles
            .iter()
            .map(|(name, lc)| (name.as_str(), lc.state()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_transitions() {
        let mut lc = ToolLifecycle::new();
        assert_eq!(lc.state(), ToolLifecycleState::Unregistered);

        lc.register().unwrap();
        assert_eq!(lc.state(), ToolLifecycleState::Registered);

        lc.enable().unwrap();
        assert_eq!(lc.state(), ToolLifecycleState::Enabled);
    }

    #[test]
    fn test_invalid_transition() {
        let lc = ToolLifecycle::new();
        let result = lc.state().transition_to(ToolLifecycleState::Enabled);
        assert!(result.is_err());
    }

    #[test]
    fn test_disable_enable_cycle() {
        let mut lc = ToolLifecycle::new();
        lc.register().unwrap();
        lc.enable().unwrap();
        lc.disable().unwrap();
        assert_eq!(lc.state(), ToolLifecycleState::Disabled);
        lc.enable().unwrap();
        assert_eq!(lc.state(), ToolLifecycleState::Enabled);
    }

    #[test]
    fn test_deprecation() {
        let mut lc = ToolLifecycle::new();
        lc.register().unwrap();
        lc.enable().unwrap();
        lc.deprecate().unwrap();
        assert_eq!(lc.state(), ToolLifecycleState::Deprecating);
        assert!(lc.state().requires_warning());
        assert!(lc.state().is_active());
    }

    #[test]
    fn test_remove() {
        let mut lc = ToolLifecycle::new();
        lc.register().unwrap();
        lc.enable().unwrap();
        lc.deprecate().unwrap();
        lc.remove().unwrap();
        assert_eq!(lc.state(), ToolLifecycleState::Removed);
        assert!(lc.state().is_terminal());
        assert!(!lc.state().is_active());
    }

    #[test]
    fn test_history() {
        let mut lc = ToolLifecycle::new();
        lc.register().unwrap();
        lc.enable().unwrap();
        assert_eq!(lc.history().len(), 2);
        assert_eq!(lc.history()[0].0, ToolLifecycleState::Registered);
        assert_eq!(lc.history()[1].0, ToolLifecycleState::Enabled);
    }

    #[test]
    fn test_lifecycle_manager() {
        let mut mgr = LifecycleManager::new();
        mgr.register("tool_a").unwrap();
        mgr.enable("tool_a").unwrap();
        assert!(mgr.is_active("tool_a"));
        assert!(!mgr.is_active("tool_b"));

        mgr.disable("tool_a").unwrap();
        assert!(!mgr.is_active("tool_a"));

        let states = mgr.all_states();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].0, "tool_a");
        assert_eq!(states[0].1, ToolLifecycleState::Disabled);
    }
}
