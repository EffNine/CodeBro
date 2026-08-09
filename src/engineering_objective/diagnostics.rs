//! Diagnostics for the engineering objective runtime.

use serde::{Deserialize, Serialize};

/// Where the current objective came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectiveSource {
    /// Loaded from a persisted `.codebro/engineering_objective.json`.
    Loaded,
    /// Constructed from the repository's documented project goals.
    Default,
    /// Created fresh (empty objective).
    Created,
}

impl std::fmt::Display for ObjectiveSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectiveSource::Loaded => write!(f, "loaded"),
            ObjectiveSource::Default => write!(f, "default"),
            ObjectiveSource::Created => write!(f, "created"),
        }
    }
}

/// Diagnostics captured by the objective runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectiveDiagnostics {
    /// Where the objective came from.
    pub source: ObjectiveSource,
    /// Time spent loading in microseconds.
    pub load_time_us: u64,
    /// Number of success criteria.
    pub success_criteria: usize,
    /// Number of non-goals.
    pub non_goals: usize,
    /// Whether the objective has meaningful content.
    pub present: bool,
}

impl ObjectiveDiagnostics {
    pub fn new(source: ObjectiveSource) -> Self {
        ObjectiveDiagnostics {
            source,
            load_time_us: 0,
            success_criteria: 0,
            non_goals: 0,
            present: false,
        }
    }

    pub fn with_load_time(mut self, us: u64) -> Self {
        self.load_time_us = us;
        self
    }

    pub fn with_counts(mut self, criteria: usize, non_goals: usize, present: bool) -> Self {
        self.success_criteria = criteria;
        self.non_goals = non_goals;
        self.present = present;
        self
    }
}

impl Default for ObjectiveDiagnostics {
    fn default() -> Self {
        ObjectiveDiagnostics::new(ObjectiveSource::Created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostics_default() {
        let d = ObjectiveDiagnostics::default();
        assert_eq!(d.source, ObjectiveSource::Created);
        assert!(!d.present);
    }

    #[test]
    fn test_diagnostics_with_counts() {
        let d = ObjectiveDiagnostics::new(ObjectiveSource::Loaded)
            .with_load_time(42)
            .with_counts(3, 2, true);
        assert_eq!(d.source, ObjectiveSource::Loaded);
        assert_eq!(d.load_time_us, 42);
        assert_eq!(d.success_criteria, 3);
        assert_eq!(d.non_goals, 2);
        assert!(d.present);
    }

    #[test]
    fn test_source_display() {
        assert_eq!(ObjectiveSource::Loaded.to_string(), "loaded");
        assert_eq!(ObjectiveSource::Default.to_string(), "default");
        assert_eq!(ObjectiveSource::Created.to_string(), "created");
    }
}
