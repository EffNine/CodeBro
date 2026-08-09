//! Updater for project identity.
//!
//! Project identity changes only when engineering state changes.
//! Examples: architecture updated, sprint completed, constraint added,
//! decision accepted, roadmap changed, module introduced.
//!
//! Do NOT update on every prompt.

use std::time::Instant;

use super::diagnostics::ProjectIdentityDiagnostics;
use super::identity::{
    DecisionStatus, EngineeringDecision, ProjectIdentity, RoadmapItem, RoadmapStatus,
};
use super::storage::ProjectIdentityStorage;
use super::validation::{validate_identity, ValidationReport};

/// Changes to apply to the project identity.
#[derive(Debug, Clone, Default)]
pub struct IdentityChanges {
    pub add_decisions: Vec<EngineeringDecision>,
    pub update_decision_status: Vec<(String, DecisionStatus)>,
    pub add_constraints: Vec<String>,
    pub add_modules: Vec<String>,
    pub add_patterns: Vec<String>,
    pub add_conventions: Vec<String>,
    pub set_sprint: Option<String>,
    pub add_roadmap_items: Vec<RoadmapItem>,
    pub complete_roadmap_item: Option<String>,
    pub add_milestone: Option<String>,
    pub update_architecture_summary: Option<String>,
    pub add_important_files: Vec<String>,
    pub add_languages: Vec<String>,
    pub add_frameworks: Vec<String>,
    pub set_build_system: Option<String>,
    pub set_package_manager: Option<String>,
    pub set_testing_framework: Option<String>,
}

impl IdentityChanges {
    pub fn new() -> Self {
        IdentityChanges::default()
    }

    /// Returns `true` when there are no changes to apply.
    pub fn is_empty(&self) -> bool {
        self.add_decisions.is_empty()
            && self.update_decision_status.is_empty()
            && self.add_constraints.is_empty()
            && self.add_modules.is_empty()
            && self.add_patterns.is_empty()
            && self.add_conventions.is_empty()
            && self.set_sprint.is_none()
            && self.add_roadmap_items.is_empty()
            && self.complete_roadmap_item.is_none()
            && self.add_milestone.is_none()
            && self.update_architecture_summary.is_none()
            && self.add_important_files.is_empty()
            && self.add_languages.is_empty()
            && self.add_frameworks.is_empty()
            && self.set_build_system.is_none()
            && self.set_package_manager.is_none()
            && self.set_testing_framework.is_none()
    }
}

/// Result of applying identity changes.
pub struct UpdateResult {
    pub identity: ProjectIdentity,
    pub diagnostics: ProjectIdentityDiagnostics,
    pub applied: bool,
}

/// Updater for project identity.
#[derive(Debug, Clone)]
pub struct ProjectIdentityUpdater {
    storage: ProjectIdentityStorage,
    update_count: u32,
}

impl ProjectIdentityUpdater {
    /// Create a new updater for the given workspace root.
    pub fn new(workspace_root: impl AsRef<std::path::Path>) -> Self {
        ProjectIdentityUpdater {
            storage: ProjectIdentityStorage::new(workspace_root),
            update_count: 0,
        }
    }

    /// Return a reference to the underlying storage.
    pub fn storage(&self) -> &ProjectIdentityStorage {
        &self.storage
    }

    /// Validate the proposed identity before any writes.
    ///
    /// Returns `Ok(())` if the identity passes all validation rules,
    /// or `Err(ValidationReport)` describing the issues.
    pub fn validate_proposal(&self, identity: &ProjectIdentity) -> Result<(), ValidationReport> {
        let report = validate_identity(identity);
        if report.is_valid() {
            Ok(())
        } else {
            Err(report)
        }
    }

    /// Apply changes to the current identity and persist all eight files.
    ///
    /// Validates the proposed identity before writing anything. If
    /// validation fails, the current identity and canonical file are
    /// left unchanged and `None` is returned.
    ///
    /// Returns `None` if there are no changes to apply.
    pub fn update(
        &mut self,
        current: &ProjectIdentity,
        changes: IdentityChanges,
    ) -> Option<UpdateResult> {
        if changes.is_empty() {
            return None;
        }

        let update_start = Instant::now();
        let mut identity = current.clone();

        // Apply each change category.
        for decision in changes.add_decisions {
            identity = identity.add_engineering_decision(decision);
        }

        for (id, status) in changes.update_decision_status {
            for dec in &mut identity.engineering_decisions {
                if dec.id == id {
                    dec.status = status.clone();
                    break;
                }
            }
        }

        for constraint in changes.add_constraints {
            identity = identity.add_known_constraint(constraint);
        }

        for module in changes.add_modules {
            identity = identity.add_knowledge_module(module);
        }

        for pattern in changes.add_patterns {
            identity = identity.add_knowledge_pattern(pattern);
        }

        for convention in changes.add_conventions {
            identity = identity.add_coding_convention(convention);
        }

        if let Some(sprint) = changes.set_sprint {
            identity = identity.with_current_sprint(sprint);
        }

        for item in changes.add_roadmap_items {
            identity = identity.add_roadmap_item(item);
        }

        if let Some(item_id) = changes.complete_roadmap_item {
            for item in &mut identity.roadmap {
                if item.id == item_id {
                    item.status = RoadmapStatus::Completed;
                    break;
                }
            }
        }

        if let Some(milestone) = changes.add_milestone {
            identity = identity.add_recent_milestone(milestone);
        }

        if let Some(summary) = changes.update_architecture_summary {
            identity = identity.with_architecture_summary(summary);
        }

        for file in changes.add_important_files {
            identity = identity.add_important_file(file);
        }

        for lang in changes.add_languages {
            identity = identity.add_language(lang);
        }

        for fw in changes.add_frameworks {
            identity = identity.with_framework(fw);
        }

        if let Some(bs) = changes.set_build_system {
            identity = identity.with_build_system(bs);
        }

        if let Some(pm) = changes.set_package_manager {
            identity = identity.with_package_manager(pm);
        }

        if let Some(tf) = changes.set_testing_framework {
            identity = identity.with_testing_framework(tf);
        }

        // Validate before writing anything.
        if let Err(report) = self.validate_proposal(&identity) {
            let update_time_us = update_start.elapsed().as_micros() as u64;
            let diagnostics =
                ProjectIdentityDiagnostics::new(super::diagnostics::IdentitySource::Loaded)
                    .with_load_time(0)
                    .with_save_time(update_time_us)
                    .with_identity_updates(self.update_count)
                    .with_validation_errors(report.issue_count() as u32);
            return Some(UpdateResult {
                identity: current.clone(),
                diagnostics,
                applied: false,
            });
        }

        // Persist the updated identity and all projections.
        let save_start = Instant::now();
        if let Err(e) = self.storage.save_all(&identity) {
            let update_time_us = update_start.elapsed().as_micros() as u64;
            self.update_count += 1;
            let diagnostics =
                ProjectIdentityDiagnostics::new(super::diagnostics::IdentitySource::Loaded)
                    .with_load_time(0)
                    .with_save_time(update_time_us)
                    .with_identity_updates(self.update_count);
            return Some(UpdateResult {
                identity: current.clone(),
                diagnostics,
                applied: false,
            });
        }
        let save_time_us = save_start.elapsed().as_micros() as u64;

        self.update_count += 1;
        let diagnostics =
            ProjectIdentityDiagnostics::new(super::diagnostics::IdentitySource::Loaded)
                .with_load_time(0)
                .with_save_time(save_time_us)
                .with_identity_updates(self.update_count);

        Some(UpdateResult {
            identity,
            diagnostics,
            applied: true,
        })
    }

    /// Current update count.
    pub fn update_count(&self) -> u32 {
        self.update_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (ProjectIdentityUpdater, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let updater = ProjectIdentityUpdater::new(tmp.path());
        (updater, tmp)
    }

    #[test]
    fn test_update_empty_changes_returns_none() {
        let (mut updater, _tmp) = setup();
        let identity = ProjectIdentity::new("test", "rust");
        let result = updater.update(&identity, IdentityChanges::new());
        assert!(result.is_none());
    }

    #[test]
    fn test_update_adds_constraint() {
        let (mut updater, _tmp) = setup();
        let identity = ProjectIdentity::new("test", "rust");
        let changes = IdentityChanges {
            add_constraints: vec!["No raw SQL".to_string()],
            ..Default::default()
        };
        let result = updater.update(&identity, changes).expect("update");
        assert!(result.applied);
        assert_eq!(result.identity.constraint_count(), 1);
        assert_eq!(result.identity.known_constraints[0], "No raw SQL");
    }

    #[test]
    fn test_update_adds_module() {
        let (mut updater, _tmp) = setup();
        let identity = ProjectIdentity::new("test", "rust");
        let changes = IdentityChanges {
            add_modules: vec!["auth".to_string(), "api".to_string()],
            ..Default::default()
        };
        let result = updater.update(&identity, changes).expect("update");
        assert!(result.applied);
        assert_eq!(result.identity.known_module_count(), 2);
        assert_eq!(result.identity.known_modules, vec!["api", "auth"]);
    }

    #[test]
    fn test_update_adds_decision() {
        let (mut updater, _tmp) = setup();
        let identity = ProjectIdentity::new("test", "rust");
        let decision =
            EngineeringDecision::new("dec-1", "Use Axum", "Use Axum for the web server", None);
        let changes = IdentityChanges {
            add_decisions: vec![decision],
            ..Default::default()
        };
        let result = updater.update(&identity, changes).expect("update");
        assert!(result.applied);
        assert_eq!(result.identity.decision_count(), 1);
        assert_eq!(
            result.identity.engineering_decisions[0].status,
            DecisionStatus::Proposed
        );
    }

    #[test]
    fn test_update_sets_sprint() {
        let (mut updater, _tmp) = setup();
        let identity = ProjectIdentity::new("test", "rust");
        let changes = IdentityChanges {
            set_sprint: Some("sprint-23".to_string()),
            ..Default::default()
        };
        let result = updater.update(&identity, changes).expect("update");
        assert!(result.applied);
        assert!(result.identity.has_sprint());
        assert_eq!(
            result.identity.current_sprint,
            Some("sprint-23".to_string())
        );
    }

    #[test]
    fn test_update_completes_roadmap_item() {
        let (mut updater, _tmp) = setup();
        let identity = ProjectIdentity::new("test", "rust").add_roadmap_item(RoadmapItem::new(
            "item-1",
            "Fix auth bug",
            None,
        ));
        let changes = IdentityChanges {
            complete_roadmap_item: Some("item-1".to_string()),
            ..Default::default()
        };
        let result = updater.update(&identity, changes).expect("update");
        assert!(result.applied);
        assert_eq!(result.identity.roadmap[0].status, RoadmapStatus::Completed);
    }

    #[test]
    fn test_update_persists_to_storage() {
        let (mut updater, _tmp) = setup();
        let identity = ProjectIdentity::new("test", "rust");
        let changes = IdentityChanges {
            add_constraints: vec!["Test constraint".to_string()],
            ..Default::default()
        };
        updater.update(&identity, changes).expect("update");
        // Reload from storage.
        let reloaded = updater.storage().load_identity().expect("reload");
        assert_eq!(reloaded.constraint_count(), 1);
        assert_eq!(reloaded.known_constraints[0], "Test constraint");
    }

    #[test]
    fn test_update_count_increments() {
        let (mut updater, _tmp) = setup();
        let identity = ProjectIdentity::new("test", "rust");
        assert_eq!(updater.update_count(), 0);
        updater
            .update(
                &identity,
                IdentityChanges {
                    add_constraints: vec!["c1".to_string()],
                    ..Default::default()
                },
            )
            .expect("update");
        assert_eq!(updater.update_count(), 1);
        updater
            .update(
                &identity,
                IdentityChanges {
                    add_modules: vec!["m1".to_string()],
                    ..Default::default()
                },
            )
            .expect("update");
        assert_eq!(updater.update_count(), 2);
    }

    #[test]
    fn test_update_adds_important_file() {
        let (mut updater, _tmp) = setup();
        let identity = ProjectIdentity::new("test", "rust");
        let changes = IdentityChanges {
            add_important_files: vec!["main.rs".to_string(), "Cargo.toml".to_string()],
            ..Default::default()
        };
        let result = updater.update(&identity, changes).expect("update");
        assert!(result.applied);
        assert_eq!(
            result.identity.important_files,
            vec!["Cargo.toml", "main.rs"]
        );
    }

    #[test]
    fn test_update_adds_language() {
        let (mut updater, _tmp) = setup();
        let identity = ProjectIdentity::new("test", "rust");
        let changes = IdentityChanges {
            add_languages: vec!["go".to_string()],
            ..Default::default()
        };
        let result = updater.update(&identity, changes).expect("update");
        assert!(result.applied);
        assert_eq!(result.identity.languages, vec!["go", "rust"]);
    }

    #[test]
    fn test_update_persists_all_projections() {
        let (mut updater, _tmp) = setup();
        let identity = ProjectIdentity::new("test", "rust").with_architecture_summary("layered");
        let changes = IdentityChanges {
            add_constraints: vec!["no-raw-sql".to_string()],
            set_sprint: Some("sprint-23".to_string()),
            ..Default::default()
        };
        updater.update(&identity, changes).expect("update");
        // All eight files should exist.
        assert!(updater.storage().identity_path().exists());
        assert!(updater.storage().workspace_path().exists());
        assert!(updater.storage().architecture_path().exists());
        assert!(updater.storage().decisions_path().exists());
        assert!(updater.storage().constraints_path().exists());
        assert!(updater.storage().roadmap_path().exists());
        assert!(updater.storage().sprint_path().exists());
        assert!(updater.storage().metadata_path().exists());
    }
}
