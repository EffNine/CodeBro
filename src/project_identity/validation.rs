//! Validation rules for `ProjectIdentity`.
//!
//! Detects:
//! - Missing required metadata (name, languages)
//! - Duplicate decision ids
//! - Duplicate roadmap item ids
//! - Invalid roadmap status values
//! - Unknown schema version

use super::identity::{DecisionStatus, ProjectIdentity, RoadmapStatus, CURRENT_SCHEMA_VERSION};

/// A single validation issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub field: String,
    pub message: String,
}

impl std::fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.field, self.message)
    }
}

/// Report of all validation issues found in an identity.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidationReport {
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    pub fn new() -> Self {
        ValidationReport {
            issues: Vec::new(),
        }
    }

    pub fn add(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.issues.push(ValidationIssue {
            field: field.into(),
            message: message.into(),
        });
    }

    /// Returns `true` when no issues were found.
    pub fn is_valid(&self) -> bool {
        self.issues.is_empty()
    }

    /// Returns the number of issues.
    pub fn issue_count(&self) -> usize {
        self.issues.len()
    }

    /// Human-readable summary.
    pub fn summary(&self) -> String {
        if self.is_valid() {
            "Identity is valid.".to_string()
        } else {
            format!(
                "Identity has {} issue(s):\n{}",
                self.issues.len(),
                self.issues
                    .iter()
                    .map(|i| format!("  - {}", i))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }
    }
}

/// Validate a `ProjectIdentity`.
///
/// Runs all deterministic validation rules and returns a `ValidationReport`.
pub fn validate_identity(identity: &ProjectIdentity) -> ValidationReport {
    let mut report = ValidationReport::new();

    // Required: name must not be empty.
    if identity.name.is_empty() {
        report.add("name", "project name is required and must not be empty");
    }

    // Required: at least one language.
    if identity.languages.is_empty() {
        report.add("languages", "at least one language is required");
    }

    // Schema version must be known.
    if identity.schema_version != CURRENT_SCHEMA_VERSION {
        report.add(
            "schema_version",
            format!(
                "unknown schema version '{}'; expected '{}'",
                identity.schema_version, CURRENT_SCHEMA_VERSION
            ),
        );
    }

    // Check for duplicate decision ids.
    let mut seen_decisions = std::collections::BTreeSet::new();
    for dec in &identity.engineering_decisions {
        if !seen_decisions.insert(&dec.id) {
            report.add(
                "engineering_decisions",
                format!("duplicate decision id: {}", dec.id),
            );
        }
    }

    // Check for duplicate roadmap item ids.
    let mut seen_roadmap = std::collections::BTreeSet::new();
    for item in &identity.roadmap {
        if !seen_roadmap.insert(&item.id) {
            report.add(
                "roadmap",
                format!("duplicate roadmap item id: {}", item.id),
            );
        }
    }

    // Validate roadmap item statuses are known values.
    for item in &identity.roadmap {
        match &item.status {
            RoadmapStatus::Planned
            | RoadmapStatus::InProgress
            | RoadmapStatus::Completed
            | RoadmapStatus::Deferred => {}
        }
    }

    // Validate decision statuses are known values.
    for dec in &identity.engineering_decisions {
        match &dec.status {
            DecisionStatus::Proposed
            | DecisionStatus::Accepted
            | DecisionStatus::Deprecated
            | DecisionStatus::Superseded => {}
        }
    }

    // Check for duplicate constraints (should be deduplicated, but validate anyway).
    let mut seen_constraints = std::collections::BTreeSet::new();
    for constraint in &identity.known_constraints {
        if !seen_constraints.insert(constraint) {
            report.add(
                "known_constraints",
                format!("duplicate constraint: {}", constraint),
            );
        }
    }

    // Check for duplicate modules.
    let mut seen_modules = std::collections::BTreeSet::new();
    for module in &identity.known_modules {
        if !seen_modules.insert(module) {
            report.add(
                "known_modules",
                format!("duplicate module: {}", module),
            );
        }
    }

    // Check for duplicate patterns.
    let mut seen_patterns = std::collections::BTreeSet::new();
    for pattern in &identity.known_patterns {
        if !seen_patterns.insert(pattern) {
            report.add(
                "known_patterns",
                format!("duplicate pattern: {}", pattern),
            );
        }
    }

    // Check for duplicate conventions.
    let mut seen_conventions = std::collections::BTreeSet::new();
    for convention in &identity.coding_conventions {
        if !seen_conventions.insert(convention) {
            report.add(
                "coding_conventions",
                format!("duplicate convention: {}", convention),
            );
        }
    }

    // Check for duplicate languages.
    let mut seen_languages = std::collections::BTreeSet::new();
    for lang in &identity.languages {
        if !seen_languages.insert(lang) {
            report.add(
                "languages",
                format!("duplicate language: {}", lang),
            );
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_identity::identity::{EngineeringDecision, ProjectIdentity, DecisionStatus};

    #[test]
    fn test_valid_identity() {
        let identity = ProjectIdentity::new("my-proj", "rust")
            .with_build_system("cargo")
            .add_known_constraint("No raw SQL")
            .add_engineering_decision(
                EngineeringDecision::new("dec-1", "Use Rust", "Use Rust", None)
                    .accept(),
            );
        let report = validate_identity(&identity);
        assert!(report.is_valid());
        assert_eq!(report.issue_count(), 0);
    }

    #[test]
    fn test_missing_name() {
        let identity = ProjectIdentity {
            name: String::new(),
            ..ProjectIdentity::new("dummy", "rust")
        };
        let report = validate_identity(&identity);
        assert!(!report.is_valid());
        assert!(report.issue_count() >= 1);
    }

    #[test]
    fn test_missing_languages() {
        let identity = ProjectIdentity {
            languages: Vec::new(),
            ..ProjectIdentity::new("dummy", "rust")
        };
        let report = validate_identity(&identity);
        assert!(!report.is_valid());
        assert!(report.issue_count() >= 1);
    }

    #[test]
    fn test_duplicate_decision_ids() {
        let identity = ProjectIdentity::new("dummy", "rust")
            .add_engineering_decision(
                EngineeringDecision::new("dec-1", "D1", "Desc", None),
            )
            .add_engineering_decision(
                EngineeringDecision::new("dec-1", "D2", "Desc2", None),
            );
        let report = validate_identity(&identity);
        assert!(!report.is_valid());
        assert!(report.issue_count() >= 1);
    }

    #[test]
    fn test_duplicate_roadmap_item_ids() {
        let identity = ProjectIdentity::new("dummy", "rust")
            .add_roadmap_item(
                crate::project_identity::identity::RoadmapItem::new("item-1", "I1", None),
            )
            .add_roadmap_item(
                crate::project_identity::identity::RoadmapItem::new("item-1", "I2", None),
            );
        let report = validate_identity(&identity);
        assert!(!report.is_valid());
        assert!(report.issue_count() >= 1);
    }

    #[test]
    fn test_unknown_schema_version() {
        let identity = ProjectIdentity {
            schema_version: "9.9.9".to_string(),
            ..ProjectIdentity::new("dummy", "rust")
        };
        let report = validate_identity(&identity);
        assert!(!report.is_valid());
        assert!(report.issue_count() >= 1);
    }

    #[test]
    fn test_summary_format() {
        let identity = ProjectIdentity {
            name: String::new(),
            languages: Vec::new(),
            ..ProjectIdentity::new("dummy", "rust")
        };
        let report = validate_identity(&identity);
        let summary = report.summary();
        assert!(summary.contains("issue"));
        assert!(summary.contains("name"));
        assert!(summary.contains("languages"));
    }
}
