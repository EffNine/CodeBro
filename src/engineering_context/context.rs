//! The core `EngineeringContext` type — the universal runtime contract.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// Machine-authoritative structured facts from a specialist result.
/// This carries the structured data (not rendered prose) so downstream
/// consumers can access machine facts without parsing prose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StructuredFacts {
    /// The source specialist that produced these facts.
    pub source: String,
    /// Key-value structured payload. The exact keys depend on the source.
    /// For research: files_inspected, symbols_found, findings_count, termination
    /// For testing: commands_run_count, failures_count, exit_codes, git_tree_unchanged
    /// For planning: steps_count, affected_files_count, affected_symbols_count, risks_count
    /// For coding: changes_count, verified_changes_count, unplanned_changes_count, verification_count, all_verified
    /// For review: findings_count, verdict, verified_changes_count, unverified_changes_count, plan_deviations_count
    pub payload: HashMap<String, serde_json::Value>,
}

impl StructuredFacts {
    /// Create empty structured facts for a source.
    pub fn new(source: impl Into<String>) -> Self {
        StructuredFacts {
            source: source.into(),
            payload: HashMap::new(),
        }
    }

    /// Insert a typed value into the payload.
    pub fn with_field(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        if let Ok(json) = serde_json::to_value(value) {
            self.payload.insert(key.into(), json);
        }
        self
    }
}

/// A single context fragment from the assembly result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextFragment {
    pub source: String,
    pub content: String,
    pub relevance_score: f64,
    /// Optional machine-authoritative structured facts from the specialist.
    /// When present, these facts originate directly from structured result
    /// fields, never from parsing rendered prose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_facts: Option<StructuredFacts>,
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

    #[test]
    fn test_structured_facts_survive_builder_insertion() {
        use crate::engineering_context::context::StructuredFacts;

        let facts = StructuredFacts::new("testing")
            .with_field("exit_code", 101i32)
            .with_field("success", false);

        let ctx = EngineeringContextBuilder::new()
            .project(ProjectIdentity::new("proj", "rust"))
            .task(IntentPlan {
                detected_goal: "test".to_string(),
                intent_type: "Execution".to_string(),
                confidence: 0.9,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .context_fragment(ContextFragment {
                source: "testing".to_string(),
                content: "tests failed".to_string(),
                relevance_score: 0.85,
                structured_facts: Some(facts.clone()),
            })
            .user_request("test")
            .system_prompt("sys")
            .build()
            .expect("build should succeed");

        assert_eq!(ctx.fragment_count(), 1);
        let frag = &ctx.context_fragments[0];
        assert_eq!(frag.source, "testing");
        let sf = frag
            .structured_facts
            .as_ref()
            .expect("facts must be present");
        assert_eq!(sf.source, "testing");
        assert_eq!(sf.payload.get("exit_code").unwrap().as_i64().unwrap(), 101);
        assert_eq!(sf.payload.get("success").unwrap().as_bool().unwrap(), false);
    }

    #[test]
    fn test_structured_facts_survive_serialization_roundtrip() {
        use crate::engineering_context::context::StructuredFacts;

        let facts = StructuredFacts::new("coding")
            .with_field("all_verified", false)
            .with_field("changes_count", 2usize);

        let mut ctx = sample_context();
        ctx.context_fragments.push(ContextFragment {
            source: "coding".to_string(),
            content: "applied 2 changes".to_string(),
            relevance_score: 0.9,
            structured_facts: Some(facts.clone()),
        });

        let json = serde_json::to_string(&ctx).expect("serialize");
        let decoded: EngineeringContext = serde_json::from_str(&json).expect("deserialize");

        let frag = decoded
            .context_fragments
            .iter()
            .find(|f| f.source == "coding")
            .expect("coding fragment must exist");
        let sf = frag
            .structured_facts
            .as_ref()
            .expect("facts must survive roundtrip");
        assert_eq!(sf.source, "coding");
        assert_eq!(
            sf.payload.get("all_verified").unwrap().as_bool().unwrap(),
            false
        );
        assert_eq!(
            sf.payload.get("changes_count").unwrap().as_u64().unwrap(),
            2
        );
    }
}
