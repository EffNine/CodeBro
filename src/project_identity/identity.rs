//! Core `ProjectIdentity` type — the engineering memory of a repository.
//!
//! `ProjectIdentity` captures architecture, decisions, constraints, and
//! evolution. It is NOT chat history. It is the persistent engineering
//! memory that makes CodeBro progressively smarter the longer it works
//! inside the same repository.
//!
//! ## What Is Stored
//!
//! | Field | Purpose |
//! |-------|---------|
//! | `name` | Project name |
//! | `description` | Human-readable summary |
//! | `languages` | Programming languages in use |
//! | `frameworks` | Frameworks and libraries |
//! | `build_system` | Build tool (cargo, make, etc.) |
//! | `package_manager` | Package manager (npm, cargo, etc.) |
//! | `testing_framework` | Test runner |
//! | `repository_url` | Remote repository URL |
//! | `repository_type` | VCS type (git, hg, etc.) |
//! | `architecture_summary` | High-level architecture description |
//! | `known_patterns` | Recognised architectural patterns |
//! | `known_modules` | Top-level module names |
//! | `important_files` | Critical project files, sorted |
//! | `engineering_decisions` | Recorded ADR-style decisions |
//! | `known_constraints` | Hard and soft constraints |
//! | `current_sprint` | Active sprint identifier |
//! | `roadmap` | Planned work items |
//! | `recent_milestones` | Completed milestones |
//! | `coding_conventions` | Style and convention rules |
//! | `workspace_root` | Absolute path to workspace root |
//! | `schema_version` | Identity schema version |
//! | `created_at` | First known session timestamp |
//! | `updated_at` | Last identity update timestamp |
//!
//! ## What Is NOT Stored
//!
//! - Conversation history
//! - LLM responses
//! - Prompt text
//! - Provider state
//! - Temporary diagnostics
//! - Runtime caches
//! - Anything session-specific

use serde::{Deserialize, Serialize};

/// Current schema version for project identity.
pub const CURRENT_SCHEMA_VERSION: &str = "1.0.0";

/// Status of an engineering decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionStatus {
    Proposed,
    Accepted,
    Deprecated,
    Superseded,
}

impl std::fmt::Display for DecisionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecisionStatus::Proposed => write!(f, "proposed"),
            DecisionStatus::Accepted => write!(f, "accepted"),
            DecisionStatus::Deprecated => write!(f, "deprecated"),
            DecisionStatus::Superseded => write!(f, "superseded"),
        }
    }
}

/// A single engineering decision recorded in project identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineeringDecision {
    /// Unique identifier for this decision.
    pub id: String,
    /// Short title.
    pub title: String,
    /// Full description of the decision.
    pub description: String,
    /// Current status.
    pub status: DecisionStatus,
    /// ISO 8601 timestamp of creation.
    pub created_at: String,
    /// Optional context that led to the decision.
    pub context: Option<String>,
}

impl EngineeringDecision {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
        context: Option<String>,
    ) -> Self {
        EngineeringDecision {
            id: id.into(),
            title: title.into(),
            description: description.into(),
            status: DecisionStatus::Proposed,
            created_at: chrono::Utc::now().to_rfc3339(),
            context,
        }
    }

    pub fn with_status(mut self, status: DecisionStatus) -> Self {
        self.status = status;
        self
    }

    pub fn accept(self) -> Self {
        self.with_status(DecisionStatus::Accepted)
    }

    pub fn deprecated(self) -> Self {
        self.with_status(DecisionStatus::Deprecated)
    }

    pub fn superseded(self, new_id: impl Into<String>) -> Self {
        EngineeringDecision {
            id: self.id,
            title: self.title,
            description: format!(
                "{} (superseded by {})",
                self.description,
                new_id.into()
            ),
            status: DecisionStatus::Superseded,
            created_at: self.created_at,
            context: self.context,
        }
    }
}

/// Status of a roadmap item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoadmapStatus {
    Planned,
    InProgress,
    Completed,
    Deferred,
}

impl std::fmt::Display for RoadmapStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoadmapStatus::Planned => write!(f, "planned"),
            RoadmapStatus::InProgress => write!(f, "in_progress"),
            RoadmapStatus::Completed => write!(f, "completed"),
            RoadmapStatus::Deferred => write!(f, "deferred"),
        }
    }
}

/// A single roadmap item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadmapItem {
    /// Unique identifier.
    pub id: String,
    /// Short title.
    pub title: String,
    /// Current status.
    pub status: RoadmapStatus,
    /// Optional sprint this belongs to.
    pub sprint: Option<String>,
    /// Optional description.
    pub description: Option<String>,
}

impl RoadmapItem {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        description: Option<String>,
    ) -> Self {
        RoadmapItem {
            id: id.into(),
            title: title.into(),
            status: RoadmapStatus::Planned,
            sprint: None,
            description,
        }
    }

    pub fn in_progress(self) -> Self {
        RoadmapItem {
            status: RoadmapStatus::InProgress,
            ..self
        }
    }

    pub fn completed(self) -> Self {
        RoadmapItem {
            status: RoadmapStatus::Completed,
            ..self
        }
    }

    pub fn with_sprint(mut self, sprint: impl Into<String>) -> Self {
        self.sprint = Some(sprint.into());
        self
    }
}

/// Immutable snapshot of project identity.
///
/// `ProjectIdentity` is immutable once built. Use
/// `ProjectIdentityBuilder` to construct or transform it.
///
/// **Note:** `PartialEq` ignores `created_at` and `updated_at`
/// timestamps, since time-dependent fields are inherently
/// non-deterministic per the Engineering Principles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectIdentity {
    /// Human-readable project name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Programming languages in use, sorted for determinism.
    pub languages: Vec<String>,
    /// Frameworks in use, sorted for determinism.
    pub frameworks: Vec<String>,
    /// Build system (e.g. "cargo", "make").
    pub build_system: Option<String>,
    /// Package manager (e.g. "cargo", "npm").
    pub package_manager: Option<String>,
    /// Testing framework (e.g. "cargo test", "jest").
    pub testing_framework: Option<String>,
    /// Remote repository URL.
    pub repository_url: Option<String>,
    /// VCS type (e.g. "git").
    pub repository_type: Option<String>,
    /// High-level architecture summary.
    pub architecture_summary: Option<String>,
    /// Recognised architectural patterns, sorted.
    pub known_patterns: Vec<String>,
    /// Top-level module names, sorted.
    pub known_modules: Vec<String>,
    /// Critical project files, sorted alphabetically.
    pub important_files: Vec<String>,
    /// Recorded engineering decisions, sorted by id.
    pub engineering_decisions: Vec<EngineeringDecision>,
    /// Hard and soft constraints, sorted.
    pub known_constraints: Vec<String>,
    /// Active sprint identifier.
    pub current_sprint: Option<String>,
    /// Planned work items, sorted by id.
    pub roadmap: Vec<RoadmapItem>,
    /// Completed milestones, sorted by reverse chronological order.
    pub recent_milestones: Vec<String>,
    /// Coding conventions, sorted.
    pub coding_conventions: Vec<String>,
    /// Absolute path to workspace root.
    pub workspace_root: Option<String>,
    /// Schema version of this identity.
    pub schema_version: String,
    /// ISO 8601 timestamp of first known session.
    pub created_at: Option<String>,
    /// ISO 8601 timestamp of last identity update.
    pub updated_at: String,
}

impl ProjectIdentity {
    /// Create a minimal project identity with just a name and language.
    pub fn new(name: impl Into<String>, language: impl Into<String>) -> Self {
        ProjectIdentity {
            name: name.into(),
            description: None,
            languages: vec![language.into()],
            frameworks: Vec::new(),
            build_system: None,
            package_manager: None,
            testing_framework: None,
            repository_url: None,
            repository_type: None,
            architecture_summary: None,
            known_patterns: Vec::new(),
            known_modules: Vec::new(),
            important_files: Vec::new(),
            engineering_decisions: Vec::new(),
            known_constraints: Vec::new(),
            current_sprint: None,
            roadmap: Vec::new(),
            recent_milestones: Vec::new(),
            coding_conventions: Vec::new(),
            workspace_root: None,
            schema_version: CURRENT_SCHEMA_VERSION.to_string(),
            created_at: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Returns `true` when the identity has only the bare minimum
    /// (`name` + single `language`) and no optional fields.
    pub fn is_basic(&self) -> bool {
        self.description.is_none()
            && self.languages.len() == 1
            && self.frameworks.is_empty()
            && self.build_system.is_none()
            && self.package_manager.is_none()
            && self.testing_framework.is_none()
            && self.repository_url.is_none()
            && self.repository_type.is_none()
            && self.architecture_summary.is_none()
            && self.known_patterns.is_empty()
            && self.known_modules.is_empty()
            && self.important_files.is_empty()
            && self.engineering_decisions.is_empty()
            && self.known_constraints.is_empty()
            && self.current_sprint.is_none()
            && self.roadmap.is_empty()
            && self.recent_milestones.is_empty()
            && self.coding_conventions.is_empty()
            && self.workspace_root.is_none()
    }

    // ── Descriptive accessors ──────────────────────────────────────────

    /// Primary language (first element of `languages`).
    pub fn primary_language(&self) -> &str {
        self.languages.first().map(|s| s.as_str()).unwrap_or("")
    }

    /// Number of engineering decisions.
    pub fn decision_count(&self) -> usize {
        self.engineering_decisions.len()
    }

    /// Number of known constraints.
    pub fn constraint_count(&self) -> usize {
        self.known_constraints.len()
    }

    /// Number of roadmap items.
    pub fn roadmap_item_count(&self) -> usize {
        self.roadmap.len()
    }

    /// Number of known modules.
    pub fn known_module_count(&self) -> usize {
        self.known_modules.len()
    }

    /// Number of important files.
    pub fn important_file_count(&self) -> usize {
        self.important_files.len()
    }

    /// Number of known patterns.
    pub fn pattern_count(&self) -> usize {
        self.known_patterns.len()
    }

    /// Number of coding conventions.
    pub fn convention_count(&self) -> usize {
        self.coding_conventions.len()
    }

    /// Returns `true` if there is an active sprint.
    pub fn has_sprint(&self) -> bool {
        self.current_sprint.is_some()
    }

    /// Returns the project age in days from `created_at`, or 0 if unknown.
    pub fn project_age_days(&self) -> u64 {
        if let Some(ref created) = self.created_at {
            if let Ok(epoch) = chrono::DateTime::parse_from_rfc3339(created) {
                let now = chrono::Utc::now();
                let duration = now.signed_duration_since(epoch);
                if duration.num_days() > 0 {
                    return duration.num_days() as u64;
                }
            }
        }
        0
    }

    /// Returns the last update timestamp as a formatted string, or "unknown".
    pub fn last_update_display(&self) -> &str {
        self.updated_at.as_str()
    }

    // ── Fluent mutators (return Self for chaining) ─────────────────────

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn with_framework(mut self, framework: impl Into<String>) -> Self {
        self.frameworks.push(framework.into());
        self.frameworks.sort();
        self.frameworks.dedup();
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn with_build_system(mut self, system: impl Into<String>) -> Self {
        self.build_system = Some(system.into());
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn with_package_manager(mut self, pm: impl Into<String>) -> Self {
        self.package_manager = Some(pm.into());
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn with_testing_framework(mut self, tf: impl Into<String>) -> Self {
        self.testing_framework = Some(tf.into());
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn add_language(mut self, language: impl Into<String>) -> Self {
        self.languages.push(language.into());
        self.languages.sort();
        self.languages.dedup();
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn add_important_file(mut self, file: impl Into<String>) -> Self {
        self.important_files.push(file.into());
        self.important_files.sort();
        self.important_files.dedup();
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn with_important_files(mut self, files: Vec<String>) -> Self {
        self.important_files = files;
        self.important_files.sort();
        self.important_files.dedup();
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn add_knowledge_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.known_patterns.push(pattern.into());
        self.known_patterns.sort();
        self.known_patterns.dedup();
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn add_knowledge_module(mut self, module: impl Into<String>) -> Self {
        self.known_modules.push(module.into());
        self.known_modules.sort();
        self.known_modules.dedup();
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn add_engineering_decision(mut self, decision: EngineeringDecision) -> Self {
        self.engineering_decisions.push(decision);
        self.engineering_decisions
            .sort_by(|a, b| a.id.cmp(&b.id));
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn add_known_constraint(mut self, constraint: impl Into<String>) -> Self {
        self.known_constraints.push(constraint.into());
        self.known_constraints.sort();
        self.known_constraints.dedup();
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn with_current_sprint(mut self, sprint: impl Into<String>) -> Self {
        self.current_sprint = Some(sprint.into());
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn add_roadmap_item(mut self, item: RoadmapItem) -> Self {
        self.roadmap.push(item);
        self.roadmap.sort_by(|a, b| a.id.cmp(&b.id));
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn add_recent_milestone(mut self, milestone: impl Into<String>) -> Self {
        self.recent_milestones.push(milestone.into());
        self.recent_milestones.sort();
        self.recent_milestones.dedup();
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn add_coding_convention(mut self, convention: impl Into<String>) -> Self {
        self.coding_conventions.push(convention.into());
        self.coding_conventions.sort();
        self.coding_conventions.dedup();
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn with_workspace_root(mut self, root: impl Into<String>) -> Self {
        self.workspace_root = Some(root.into());
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }

    pub fn with_architecture_summary(mut self, summary: impl Into<String>) -> Self {
        self.architecture_summary = Some(summary.into());
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self
    }
}

impl Default for ProjectIdentity {
    fn default() -> Self {
        ProjectIdentity::new("unknown", "unknown")
    }
}

impl PartialEq for ProjectIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.description == other.description
            && self.languages == other.languages
            && self.frameworks == other.frameworks
            && self.build_system == other.build_system
            && self.package_manager == other.package_manager
            && self.testing_framework == other.testing_framework
            && self.repository_url == other.repository_url
            && self.repository_type == other.repository_type
            && self.architecture_summary == other.architecture_summary
            && self.known_patterns == other.known_patterns
            && self.known_modules == other.known_modules
            && self.important_files == other.important_files
            && self.engineering_decisions == other.engineering_decisions
            && self.known_constraints == other.known_constraints
            && self.current_sprint == other.current_sprint
            && self.roadmap == other.roadmap
            && self.recent_milestones == other.recent_milestones
            && self.coding_conventions == other.coding_conventions
            && self.workspace_root == other.workspace_root
            && self.schema_version == other.schema_version
            // Intentionally ignore created_at and updated_at.
    }
}

impl Eq for ProjectIdentity {}
