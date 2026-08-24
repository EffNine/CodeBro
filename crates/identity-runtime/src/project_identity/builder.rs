//! Builder for `ProjectIdentity`.
//!
//! Validates required metadata before build.

use super::identity::{DecisionStatus, EngineeringDecision, ProjectIdentity, RoadmapItem};

/// Errors that can occur during identity construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityBuildError {
    /// Project name is required.
    MissingName,
    /// At least one language is required.
    MissingLanguage,
    /// Duplicate decision id detected.
    DuplicateDecisionId(String),
    /// Duplicate roadmap item id detected.
    DuplicateRoadmapItemId(String),
}

impl std::fmt::Display for IdentityBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityBuildError::MissingName => write!(f, "project name is required"),
            IdentityBuildError::MissingLanguage => write!(f, "at least one language is required"),
            IdentityBuildError::DuplicateDecisionId(id) => {
                write!(f, "duplicate decision id: {}", id)
            }
            IdentityBuildError::DuplicateRoadmapItemId(id) => {
                write!(f, "duplicate roadmap item id: {}", id)
            }
        }
    }
}

impl std::error::Error for IdentityBuildError {}

/// Fluent builder for `ProjectIdentity`.
///
/// Validates required fields at build time.
#[derive(Debug, Default)]
pub struct ProjectIdentityBuilder {
    name: Option<String>,
    description: Option<String>,
    languages: Vec<String>,
    frameworks: Vec<String>,
    build_system: Option<String>,
    package_manager: Option<String>,
    testing_framework: Option<String>,
    repository_url: Option<String>,
    repository_type: Option<String>,
    architecture_summary: Option<String>,
    known_patterns: Vec<String>,
    known_modules: Vec<String>,
    important_files: Vec<String>,
    engineering_decisions: Vec<EngineeringDecision>,
    known_constraints: Vec<String>,
    current_sprint: Option<String>,
    roadmap: Vec<RoadmapItem>,
    recent_milestones: Vec<String>,
    coding_conventions: Vec<String>,
    workspace_root: Option<String>,
    schema_version: String,
}

impl ProjectIdentityBuilder {
    pub fn new() -> Self {
        ProjectIdentityBuilder {
            schema_version: super::identity::CURRENT_SCHEMA_VERSION.to_string(),
            ..Default::default()
        }
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.languages.push(language.into());
        self.languages.sort();
        self.languages.dedup();
        self
    }

    pub fn languages(mut self, languages: Vec<String>) -> Self {
        self.languages = languages;
        self.languages.sort();
        self.languages.dedup();
        self
    }

    pub fn framework(mut self, framework: impl Into<String>) -> Self {
        self.frameworks.push(framework.into());
        self.frameworks.sort();
        self.frameworks.dedup();
        self
    }

    pub fn build_system(mut self, system: impl Into<String>) -> Self {
        self.build_system = Some(system.into());
        self
    }

    pub fn package_manager(mut self, pm: impl Into<String>) -> Self {
        self.package_manager = Some(pm.into());
        self
    }

    pub fn testing_framework(mut self, tf: impl Into<String>) -> Self {
        self.testing_framework = Some(tf.into());
        self
    }

    pub fn repository_url(mut self, url: impl Into<String>) -> Self {
        self.repository_url = Some(url.into());
        self
    }

    pub fn repository_type(mut self, typ: impl Into<String>) -> Self {
        self.repository_type = Some(typ.into());
        self
    }

    pub fn architecture_summary(mut self, summary: impl Into<String>) -> Self {
        self.architecture_summary = Some(summary.into());
        self
    }

    pub fn known_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.known_patterns.push(pattern.into());
        self.known_patterns.sort();
        self.known_patterns.dedup();
        self
    }

    pub fn known_module(mut self, module: impl Into<String>) -> Self {
        self.known_modules.push(module.into());
        self.known_modules.sort();
        self.known_modules.dedup();
        self
    }

    pub fn important_file(mut self, file: impl Into<String>) -> Self {
        self.important_files.push(file.into());
        self.important_files.sort();
        self.important_files.dedup();
        self
    }

    pub fn important_files(mut self, files: Vec<String>) -> Self {
        self.important_files = files;
        self.important_files.sort();
        self.important_files.dedup();
        self
    }

    pub fn engineering_decision(mut self, decision: EngineeringDecision) -> Self {
        self.engineering_decisions.push(decision);
        self.engineering_decisions.sort_by(|a, b| a.id.cmp(&b.id));
        self
    }

    pub fn known_constraint(mut self, constraint: impl Into<String>) -> Self {
        self.known_constraints.push(constraint.into());
        self.known_constraints.sort();
        self.known_constraints.dedup();
        self
    }

    pub fn current_sprint(mut self, sprint: impl Into<String>) -> Self {
        self.current_sprint = Some(sprint.into());
        self
    }

    pub fn roadmap_item(mut self, item: RoadmapItem) -> Self {
        self.roadmap.push(item);
        self.roadmap.sort_by(|a, b| a.id.cmp(&b.id));
        self
    }

    pub fn recent_milestone(mut self, milestone: impl Into<String>) -> Self {
        self.recent_milestones.push(milestone.into());
        self.recent_milestones.sort();
        self.recent_milestones.dedup();
        self
    }

    pub fn coding_convention(mut self, convention: impl Into<String>) -> Self {
        self.coding_conventions.push(convention.into());
        self.coding_conventions.sort();
        self.coding_conventions.dedup();
        self
    }

    pub fn workspace_root(mut self, root: impl Into<String>) -> Self {
        self.workspace_root = Some(root.into());
        self
    }

    pub fn schema_version(mut self, version: impl Into<String>) -> Self {
        self.schema_version = version.into();
        self
    }

    /// Build the `ProjectIdentity`, validating required metadata.
    pub fn build(self) -> Result<ProjectIdentity, IdentityBuildError> {
        if self.name.is_none() {
            return Err(IdentityBuildError::MissingName);
        }
        if self.languages.is_empty() {
            return Err(IdentityBuildError::MissingLanguage);
        }

        // Check for duplicate decision ids.
        let mut seen_decisions = std::collections::BTreeSet::new();
        for dec in &self.engineering_decisions {
            if !seen_decisions.insert(&dec.id) {
                return Err(IdentityBuildError::DuplicateDecisionId(dec.id.clone()));
            }
        }

        // Check for duplicate roadmap item ids.
        let mut seen_roadmap = std::collections::BTreeSet::new();
        for item in &self.roadmap {
            if !seen_roadmap.insert(&item.id) {
                return Err(IdentityBuildError::DuplicateRoadmapItemId(item.id.clone()));
            }
        }

        Ok(ProjectIdentity {
            name: self.name.unwrap(),
            description: self.description,
            languages: self.languages,
            frameworks: self.frameworks,
            build_system: self.build_system,
            package_manager: self.package_manager,
            testing_framework: self.testing_framework,
            repository_url: self.repository_url,
            repository_type: self.repository_type,
            architecture_summary: self.architecture_summary,
            known_patterns: self.known_patterns,
            known_modules: self.known_modules,
            important_files: self.important_files,
            engineering_decisions: self.engineering_decisions,
            known_constraints: self.known_constraints,
            current_sprint: self.current_sprint,
            roadmap: self.roadmap,
            recent_milestones: self.recent_milestones,
            coding_conventions: self.coding_conventions,
            workspace_root: self.workspace_root,
            schema_version: self.schema_version,
            created_at: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_identity::identity::EngineeringDecision;

    #[test]
    fn test_builder_missing_name() {
        let result = ProjectIdentityBuilder::new().language("rust").build();
        assert_eq!(result.unwrap_err(), IdentityBuildError::MissingName);
    }

    #[test]
    fn test_builder_missing_language() {
        let result = ProjectIdentityBuilder::new().name("test-proj").build();
        assert_eq!(result.unwrap_err(), IdentityBuildError::MissingLanguage);
    }

    #[test]
    fn test_builder_valid() {
        let id = ProjectIdentityBuilder::new()
            .name("my-project")
            .language("rust")
            .framework("tokio")
            .build()
            .expect("build should succeed");
        assert_eq!(id.name, "my-project");
        assert_eq!(id.primary_language(), "rust");
        assert_eq!(id.frameworks, vec!["tokio"]);
    }

    #[test]
    fn test_builder_duplicate_decision_id() {
        let decision = EngineeringDecision::new("dec-1", "Use Rust", "Use Rust for the core", None);
        let result = ProjectIdentityBuilder::new()
            .name("proj")
            .language("rust")
            .engineering_decision(decision.clone())
            .engineering_decision(decision)
            .build();
        match result {
            Err(IdentityBuildError::DuplicateDecisionId(ref id)) => {
                assert_eq!(id, "dec-1");
            }
            other => panic!("Expected DuplicateDecisionId error, got {:?}", other),
        }
    }

    #[test]
    fn test_builder_duplicate_roadmap_item_id() {
        let item = RoadmapItem::new("item-1", "Fix bug", None);
        let result = ProjectIdentityBuilder::new()
            .name("proj")
            .language("rust")
            .roadmap_item(item.clone())
            .roadmap_item(item)
            .build();
        match result {
            Err(IdentityBuildError::DuplicateRoadmapItemId(ref id)) => {
                assert_eq!(id, "item-1");
            }
            other => panic!("Expected DuplicateRoadmapItemId error, got {:?}", other),
        }
    }

    #[test]
    fn test_builder_deterministic_ordering() {
        let id = ProjectIdentityBuilder::new()
            .name("proj")
            .language("go")
            .language("rust")
            .language("python")
            .known_module("z-module")
            .known_module("a-module")
            .known_module("m-module")
            .important_file("z.rs")
            .important_file("a.rs")
            .build()
            .expect("build should succeed");
        assert_eq!(id.languages, vec!["go", "python", "rust"]);
        assert_eq!(id.known_modules, vec!["a-module", "m-module", "z-module"]);
        assert_eq!(id.important_files, vec!["a.rs", "z.rs"]);
    }

    #[test]
    fn test_builder_serialization_roundtrip() {
        let id = ProjectIdentityBuilder::new()
            .name("full-proj")
            .language("typescript")
            .framework("nextjs")
            .build_system("npm")
            .package_manager("npm")
            .testing_framework("vitest")
            .architecture_summary("Monorepo with shared libs")
            .known_pattern("clean architecture")
            .known_module("auth")
            .known_module("api")
            .important_file("package.json")
            .important_file("next.config.js")
            .known_constraint("No raw SQL")
            .known_constraint("Use context for timeouts")
            .coding_convention("PascalCase components")
            .coding_convention("kebab-case filenames")
            .build()
            .expect("build should succeed");

        let json = serde_json::to_string(&id).expect("serialize");
        let decoded: ProjectIdentity = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id.name, decoded.name);
        assert_eq!(id.languages, decoded.languages);
        assert_eq!(id.frameworks, decoded.frameworks);
        assert_eq!(id.known_modules, decoded.known_modules);
        assert_eq!(id.known_constraints, decoded.known_constraints);
        assert_eq!(id.coding_conventions, decoded.coding_conventions);
        assert_eq!(id.schema_version, decoded.schema_version);
    }
}
