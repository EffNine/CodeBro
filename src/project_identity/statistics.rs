//! Statistics for `ProjectIdentity`.
//!
//! Exposes aggregate metrics derived from the identity state.

use serde::{Deserialize, Serialize};

/// Aggregate statistics derived from a `ProjectIdentity`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectIdentityStatistics {
    /// Age of the project in days (0 if unknown).
    pub project_age_days: u64,
    /// Number of engineering decisions recorded.
    pub decision_count: usize,
    /// Number of known constraints.
    pub constraint_count: usize,
    /// Number of roadmap items.
    pub roadmap_items: usize,
    /// Number of known modules.
    pub known_modules: usize,
    /// Last update timestamp string.
    pub last_update: String,
    /// Schema version.
    pub schema_version: String,
    /// Number of important files.
    pub important_file_count: usize,
    /// Number of known patterns.
    pub pattern_count: usize,
    /// Number of coding conventions.
    pub convention_count: usize,
    /// Number of recent milestones.
    pub milestone_count: usize,
}

impl ProjectIdentityStatistics {
    pub fn from_identity(identity: &crate::project_identity::identity::ProjectIdentity) -> Self {
        ProjectIdentityStatistics {
            project_age_days: identity.project_age_days(),
            decision_count: identity.decision_count(),
            constraint_count: identity.constraint_count(),
            roadmap_items: identity.roadmap_item_count(),
            known_modules: identity.known_module_count(),
            last_update: identity.last_update_display().to_string(),
            schema_version: identity.schema_version.clone(),
            important_file_count: identity.important_file_count(),
            pattern_count: identity.pattern_count(),
            convention_count: identity.convention_count(),
            milestone_count: identity.recent_milestones.len(),
        }
    }

    /// Returns `true` when all counts are zero and no temporal data exists.
    pub fn is_empty(&self) -> bool {
        self.project_age_days == 0
            && self.decision_count == 0
            && self.constraint_count == 0
            && self.roadmap_items == 0
            && self.known_modules == 0
            && self.important_file_count == 0
            && self.pattern_count == 0
            && self.convention_count == 0
            && self.milestone_count == 0
    }
}

impl Default for ProjectIdentityStatistics {
    fn default() -> Self {
        ProjectIdentityStatistics {
            project_age_days: 0,
            decision_count: 0,
            constraint_count: 0,
            roadmap_items: 0,
            known_modules: 0,
            last_update: String::new(),
            schema_version: crate::project_identity::identity::CURRENT_SCHEMA_VERSION
                .to_string(),
            important_file_count: 0,
            pattern_count: 0,
            convention_count: 0,
            milestone_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_identity::identity::ProjectIdentity;

    #[test]
    fn test_empty_statistics() {
        let id = ProjectIdentity::new("test", "rust");
        let stats = ProjectIdentityStatistics::from_identity(&id);
        assert!(stats.is_empty());
        assert_eq!(stats.decision_count, 0);
        assert_eq!(stats.schema_version, "1.0.0");
    }

    #[test]
    fn test_statistics_with_data() {
        let id = ProjectIdentity::new("test", "rust")
            .with_description("A test project")
            .with_build_system("cargo")
            .add_knowledge_module("auth")
            .add_knowledge_module("api")
            .add_known_constraint("No raw SQL")
            .add_known_constraint("Use context for timeouts");
        let stats = ProjectIdentityStatistics::from_identity(&id);
        assert!(!stats.is_empty());
        assert_eq!(stats.known_modules, 2);
        assert_eq!(stats.constraint_count, 2);
        assert_eq!(stats.schema_version, "1.0.0");
    }

    #[test]
    fn test_serialization_roundtrip() {
        let id = ProjectIdentity::new("proj", "go")
            .add_knowledge_module("web")
            .add_knowledge_module("db");
        let stats = ProjectIdentityStatistics::from_identity(&id);
        let json = serde_json::to_string(&stats).expect("serialize");
        let decoded: ProjectIdentityStatistics =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(stats.known_modules, decoded.known_modules);
        assert_eq!(stats.schema_version, decoded.schema_version);
    }
}
