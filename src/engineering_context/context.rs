//! The core `EngineeringContext` type — the universal runtime contract.

use serde::{Deserialize, Serialize};

use super::constraints::ConstraintContext;
use super::diagnostics::EngineeringContextDiagnostics;
use super::identity::ProjectIdentity;
use super::memory::EngineeringMemoryContext;
use super::runtime::RuntimeContext;
use super::statistics::EngineeringContextStatistics;
use super::workspace::WorkspaceContext;
use crate::engineering_objective::{EngineeringObjective, GoalAlignment};

/// Intent classification result carried from context assembly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentPlan {
    pub detected_goal: String,
    pub intent_type: String,
    pub confidence: f64,
    pub ambiguity: bool,
    pub ambiguity_reason: Option<String>,
}

/// A single context fragment from the assembly result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextFragment {
    pub source: String,
    pub content: String,
    pub relevance_score: f64,
}

/// A single conversation message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
}

/// A single diagnostic event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticEvent {
    pub severity: String,
    pub message: String,
}

/// The canonical engineering context shared by all subsystems.
///
/// `EngineeringContext` is immutable once built. Subsystems read from it;
/// they never mutate it. Use `EngineeringContextBuilder` to construct or
/// transform it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineeringContext {
    /// Project identity.
    pub project: ProjectIdentity,
    /// Current task / intent.
    pub task: Option<IntentPlan>,
    /// Project objective hierarchy (compact).
    pub objective: EngineeringObjective,
    /// Goal alignment of the current task (compact metadata).
    pub goal_alignment: Option<GoalAlignment>,
    /// Workspace metadata and relevant files.
    pub workspace: WorkspaceContext,
    /// Assembled context fragments.
    pub context_fragments: Vec<ContextFragment>,
    /// Engineering memory entries.
    pub memory: EngineeringMemoryContext,
    /// Architecture and engineering constraints.
    pub constraints: ConstraintContext,
    /// Runtime metadata (provider, budget, etc.).
    pub runtime: RuntimeContext,
    /// Active file paths.
    pub active_files: Vec<String>,
    /// User request string.
    pub user_request: String,
    /// Conversation history.
    pub conversation: Vec<ConversationMessage>,
    /// System prompt.
    pub system_prompt: String,
    /// Diagnostics captured at build time.
    pub diagnostics: EngineeringContextDiagnostics,
    /// Statistics captured at build time.
    pub statistics: EngineeringContextStatistics,
}

impl EngineeringContext {
    /// Returns `true` if the context has no meaningful content.
    pub fn is_empty(&self) -> bool {
        self.user_request.is_empty()
            && self.context_fragments.is_empty()
            && self.memory.is_empty()
            && self.constraints.is_empty()
            && self.active_files.is_empty()
            && self.conversation.is_empty()
            && self.objective.is_empty()
    }

    /// Estimated total token count across all content.
    pub fn estimated_tokens(&self) -> usize {
        let fragment_tokens: usize = self
            .context_fragments
            .iter()
            .map(|f| f.content.len() / 4)
            .sum();
        let memory_tokens: usize = self
            .memory
            .entries
            .iter()
            .map(|e| format!("{}: {}", e.key, e.value).len() / 4)
            .sum();
        let conversation_tokens: usize = self
            .conversation
            .iter()
            .map(|m| format!("[{}]: {}", m.role, m.content).len() / 4)
            .sum();
        fragment_tokens
            + memory_tokens
            + conversation_tokens
            + self.user_request.len() / 4
            + self.system_prompt.len() / 4
            + self.objective.estimated_tokens()
    }

    /// Returns the number of context fragments.
    pub fn fragment_count(&self) -> usize {
        self.context_fragments.len()
    }

    /// Returns the number of memory entries.
    pub fn memory_count(&self) -> usize {
        self.memory.entry_count()
    }

    /// Returns the number of constraints.
    pub fn constraint_count(&self) -> usize {
        self.constraints.constraint_count()
    }

    /// Returns the number of workspace files.
    pub fn workspace_file_count(&self) -> usize {
        self.workspace.file_count()
    }

    /// Deterministic equality — every field is compared for equality.
    pub fn equals(&self, other: &EngineeringContext) -> bool {
        self.project == other.project
            && self.task == other.task
            && self.objective == other.objective
            && self.goal_alignment == other.goal_alignment
            && self.workspace == other.workspace
            && self.context_fragments == other.context_fragments
            && self.memory == other.memory
            && self.constraints == other.constraints
            && self.runtime == other.runtime
            && self.active_files == other.active_files
            && self.user_request == other.user_request
            && self.conversation == other.conversation
            && self.system_prompt == other.system_prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineering_context::builder::EngineeringContextBuilder;
    use crate::engineering_context::constraints::{ConstraintCategory, EngineeringConstraint};
    use crate::engineering_context::identity::ProjectIdentity;
    use crate::engineering_context::memory::{MemoryEntry, MemoryTier};
    use crate::engineering_context::workspace::WorkspaceFile;

    fn sample_context() -> EngineeringContext {
        EngineeringContextBuilder::new()
            .project(ProjectIdentity::new("test-project", "rust"))
            .workspace(
                WorkspaceContext::new(".")
                    .with_file(WorkspaceFile {
                        path: "src/main.rs".to_string(),
                        language: "rust".to_string(),
                        size_bytes: 512,
                    })
                    .with_git(true),
            )
            .memory(
                EngineeringMemoryContext::new()
                    .with_entries(vec![MemoryEntry {
                        key: "language".to_string(),
                        value: "rust".to_string(),
                        confidence: 0.95,
                        tier: MemoryTier::Project,
                    }])
                    .with_budget(1000),
            )
            .constraints(
                ConstraintContext::new().add_constraint(EngineeringConstraint {
                    description: "No raw SQL".to_string(),
                    category: ConstraintCategory::Architecture,
                }),
            )
            .task(IntentPlan {
                detected_goal: "fix bug".to_string(),
                intent_type: "Execution".to_string(),
                confidence: 0.9,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .user_request("Fix the auth bug")
            .system_prompt("You are CodeBro")
            .build()
            .expect("build should succeed")
    }

    #[test]
    fn test_empty_context() {
        let ctx = EngineeringContextBuilder::new()
            .with_skip_validation()
            .build()
            .expect("build should succeed");
        assert!(ctx.is_empty());
    }

    #[test]
    fn test_sample_context_not_empty() {
        let ctx = sample_context();
        assert!(!ctx.is_empty());
        assert_eq!(ctx.fragment_count(), 0);
        assert_eq!(ctx.memory_count(), 1);
        assert_eq!(ctx.constraint_count(), 1);
        assert_eq!(ctx.workspace_file_count(), 1);
        assert_eq!(ctx.user_request, "Fix the auth bug");
    }

    #[test]
    fn test_estimated_tokens_positive() {
        let ctx = sample_context();
        assert!(ctx.estimated_tokens() > 0);
    }

    #[test]
    fn test_deterministic_equality() {
        let ctx1 = sample_context();
        let ctx2 = sample_context();
        assert!(ctx1.equals(&ctx2));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let ctx = sample_context();
        let json = serde_json::to_string(&ctx).expect("serialize");
        let decoded: EngineeringContext = serde_json::from_str(&json).expect("deserialize");
        assert!(ctx.equals(&decoded));
    }

    #[test]
    fn test_diagnostics_present() {
        let ctx = sample_context();
        assert!(!ctx.diagnostics.creation_time.is_empty());
    }

    #[test]
    fn test_objective_roundtrip_through_serialization() {
        use crate::engineering_objective::EngineeringObjective;

        let objective = EngineeringObjective::new("End goal", "Vision", "Objective", "Milestone");
        let ctx = EngineeringContextBuilder::new()
            .with_skip_validation()
            .project(ProjectIdentity::new("proj", "rust"))
            .task(IntentPlan {
                detected_goal: "task".to_string(),
                intent_type: "Execution".to_string(),
                confidence: 0.9,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .objective(objective.clone())
            .user_request("task")
            .system_prompt("sys")
            .build()
            .expect("build");

        assert_eq!(ctx.objective, objective);

        let json = serde_json::to_string(&ctx).expect("serialize");
        let decoded: EngineeringContext = serde_json::from_str(&json).expect("deserialize");
        assert!(ctx.equals(&decoded));
        assert_eq!(decoded.objective, objective);
    }

    #[test]
    fn test_objective_participates_in_equality() {
        use crate::engineering_objective::EngineeringObjective;

        let with_objective = EngineeringContextBuilder::new()
            .with_skip_validation()
            .project(ProjectIdentity::new("proj", "rust"))
            .task(IntentPlan {
                detected_goal: "task".to_string(),
                intent_type: "Execution".to_string(),
                confidence: 0.9,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .objective(EngineeringObjective::new("g", "v", "o", "m"))
            .user_request("task")
            .system_prompt("sys")
            .build()
            .expect("build");

        let without_objective = EngineeringContextBuilder::new()
            .with_skip_validation()
            .project(ProjectIdentity::new("proj", "rust"))
            .task(IntentPlan {
                detected_goal: "task".to_string(),
                intent_type: "Execution".to_string(),
                confidence: 0.9,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .user_request("task")
            .system_prompt("sys")
            .build()
            .expect("build");

        // Different objectives must not compare equal.
        assert!(!with_objective.equals(&without_objective));
    }
}
