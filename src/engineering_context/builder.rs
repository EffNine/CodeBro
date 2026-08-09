//! Builder for `EngineeringContext`.
//!
//! Supports fluent chaining and validates required fields at build time.

use std::time::Instant;

use chrono::Utc;

use super::constraints::ConstraintContext;
use super::context::{ContextFragment, ConversationMessage, EngineeringContext, IntentPlan};
use super::diagnostics::EngineeringContextDiagnostics;
use super::identity::ProjectIdentity;
use super::memory::EngineeringMemoryContext;
use super::runtime::RuntimeContext;
use super::statistics::EngineeringContextStatistics;
use super::workspace::WorkspaceContext;
use crate::engineering_objective::{EngineeringObjective, GoalAlignment};

/// Errors that can occur during context construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextBuildError {
    MissingProjectIdentity,
    EmptyTask,
    InvalidWorkspace,
    DuplicateFragment(String),
}

impl std::fmt::Display for ContextBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextBuildError::MissingProjectIdentity => {
                write!(f, "project identity is required")
            }
            ContextBuildError::EmptyTask => {
                write!(f, "task (intent plan) is required")
            }
            ContextBuildError::InvalidWorkspace => {
                write!(f, "workspace root path is empty")
            }
            ContextBuildError::DuplicateFragment(fragment) => {
                write!(f, "duplicate fragment detected: {}", fragment)
            }
        }
    }
}

impl std::error::Error for ContextBuildError {}

/// Fluent builder for `EngineeringContext`.
#[derive(Debug, Clone, Default)]
pub struct EngineeringContextBuilder {
    project: Option<ProjectIdentity>,
    task: Option<IntentPlan>,
    objective: Option<EngineeringObjective>,
    goal_alignment: Option<GoalAlignment>,
    workspace: Option<WorkspaceContext>,
    context_fragments: Vec<ContextFragment>,
    memory: Option<EngineeringMemoryContext>,
    constraints: Option<ConstraintContext>,
    runtime: Option<RuntimeContext>,
    active_files: Vec<String>,
    user_request: String,
    conversation: Vec<ConversationMessage>,
    system_prompt: String,
    skip_validation: bool,
}

impl EngineeringContextBuilder {
    pub fn new() -> Self {
        EngineeringContextBuilder::default()
    }

    pub fn with_skip_validation(mut self) -> Self {
        self.skip_validation = true;
        self
    }

    pub fn project(mut self, project: ProjectIdentity) -> Self {
        self.project = Some(project);
        self
    }

    pub fn task(mut self, task: IntentPlan) -> Self {
        self.task = Some(task);
        self
    }

    pub fn objective(mut self, objective: EngineeringObjective) -> Self {
        self.objective = Some(objective);
        self
    }

    pub fn goal_alignment(mut self, alignment: Option<GoalAlignment>) -> Self {
        self.goal_alignment = alignment;
        self
    }

    pub fn workspace(mut self, workspace: WorkspaceContext) -> Self {
        self.workspace = Some(workspace);
        self
    }

    pub fn context_fragment(mut self, fragment: ContextFragment) -> Self {
        self.context_fragments.push(fragment);
        self
    }

    pub fn context_fragments(mut self, fragments: Vec<ContextFragment>) -> Self {
        self.context_fragments = fragments;
        self
    }

    pub fn memory(mut self, memory: EngineeringMemoryContext) -> Self {
        self.memory = Some(memory);
        self
    }

    pub fn constraints(mut self, constraints: ConstraintContext) -> Self {
        self.constraints = Some(constraints);
        self
    }

    pub fn runtime(mut self, runtime: RuntimeContext) -> Self {
        self.runtime = Some(runtime);
        self
    }

    pub fn active_file(mut self, file: impl Into<String>) -> Self {
        self.active_files.push(file.into());
        self.active_files.sort();
        self.active_files.dedup();
        self
    }

    pub fn active_files(mut self, files: Vec<String>) -> Self {
        self.active_files = files;
        self.active_files.sort();
        self.active_files.dedup();
        self
    }

    pub fn user_request(mut self, request: impl Into<String>) -> Self {
        self.user_request = request.into();
        self
    }

    pub fn conversation(mut self, messages: Vec<ConversationMessage>) -> Self {
        self.conversation = messages;
        self
    }

    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    /// Build the `EngineeringContext`, validating required fields.
    pub fn build(self) -> Result<EngineeringContext, ContextBuildError> {
        let build_start = Instant::now();
        let creation_time = Utc::now().to_rfc3339();

        if !self.skip_validation {
            self.validate()?;
        }

        let project = self
            .project
            .clone()
            .unwrap_or_else(|| ProjectIdentity::new("unknown", "unknown"));

        let objective = self.objective.clone().unwrap_or_default();
        let goal_alignment = self.goal_alignment;

        let workspace = self
            .workspace
            .clone()
            .unwrap_or_else(|| WorkspaceContext::new("."));

        let memory = self
            .memory
            .clone()
            .unwrap_or_else(EngineeringMemoryContext::new);

        let constraints = self
            .constraints
            .clone()
            .unwrap_or_else(ConstraintContext::new);

        let runtime = self.runtime.clone().unwrap_or_else(RuntimeContext::new);

        let build_duration_ms = build_start.elapsed().as_millis() as u64;

        let estimated_tokens = estimate_context_tokens(
            &self.context_fragments,
            &memory,
            &self.user_request,
            &self.system_prompt,
            &self.conversation,
        );

        let diagnostics = EngineeringContextDiagnostics::new(
            creation_time.clone(),
            build_duration_ms,
            self.context_fragments.len(),
            memory.entry_count(),
            constraints.constraint_count(),
            workspace.file_count(),
            estimated_tokens,
        )
        .with_provider(runtime.provider_name().map(|s| s.to_string()))
        .with_template(self.task.as_ref().map(|t| t.intent_type.clone()));

        let statistics = EngineeringContextStatistics::new()
            .with_file_count(workspace.file_count())
            .with_memory_entries(memory.entry_count())
            .with_constraint_entries(constraints.constraint_count())
            .with_workspace_size(workspace.total_size_bytes())
            .with_context_fragments(self.context_fragments.len())
            .with_estimated_tokens(estimated_tokens)
            .with_compile_time(build_duration_ms);

        Ok(EngineeringContext {
            project,
            task: self.task,
            objective,
            goal_alignment,
            workspace,
            context_fragments: self.context_fragments,
            memory,
            constraints,
            runtime,
            active_files: self.active_files,
            user_request: self.user_request,
            conversation: self.conversation,
            system_prompt: self.system_prompt,
            diagnostics,
            statistics,
        })
    }

    fn validate(&self) -> Result<(), ContextBuildError> {
        if self.project.is_none() {
            return Err(ContextBuildError::MissingProjectIdentity);
        }
        if self.task.is_none() {
            return Err(ContextBuildError::EmptyTask);
        }
        if let Some(ref ws) = self.workspace {
            if ws.root_path.is_empty() {
                return Err(ContextBuildError::InvalidWorkspace);
            }
        }
        // Check for duplicate fragments by source+content fingerprint.
        let mut seen = std::collections::BTreeSet::new();
        for frag in &self.context_fragments {
            let fingerprint = format!("{}:{}", frag.source, frag.content.len());
            if !seen.insert(fingerprint) {
                return Err(ContextBuildError::DuplicateFragment(frag.source.clone()));
            }
        }
        Ok(())
    }
}

fn estimate_context_tokens(
    fragments: &[ContextFragment],
    memory: &EngineeringMemoryContext,
    user_request: &str,
    system_prompt: &str,
    conversation: &[ConversationMessage],
) -> usize {
    let fragment_tokens: usize = fragments.iter().map(|f| f.content.len() / 4).sum();
    let memory_tokens: usize = memory
        .entries
        .iter()
        .map(|e| format!("{}: {}", e.key, e.value).len() / 4)
        .sum();
    let conversation_tokens: usize = conversation
        .iter()
        .map(|m| format!("[{}]: {}", m.role, m.content).len() / 4)
        .sum();
    fragment_tokens
        + memory_tokens
        + conversation_tokens
        + user_request.len() / 4
        + system_prompt.len() / 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineering_context::{
        constraints::{ConstraintCategory, EngineeringConstraint},
        identity::ProjectIdentity,
        memory::{MemoryEntry, MemoryTier},
        workspace::WorkspaceFile,
    };

    fn valid_builder() -> EngineeringContextBuilder {
        EngineeringContextBuilder::new()
            .project(ProjectIdentity::new("test-project", "rust"))
            .task(IntentPlan {
                detected_goal: "fix bug".to_string(),
                intent_type: "Execution".to_string(),
                confidence: 0.9,
                ambiguity: false,
                ambiguity_reason: None,
            })
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
                        key: "lang".to_string(),
                        value: "rust".to_string(),
                        confidence: 0.95,
                        tier: MemoryTier::Project,
                    }])
                    .with_budget(500),
            )
            .constraints(
                ConstraintContext::new().add_constraint(EngineeringConstraint {
                    description: "No raw SQL".to_string(),
                    category: ConstraintCategory::Architecture,
                }),
            )
            .user_request("Fix the auth bug")
            .system_prompt("You are CodeBro")
    }

    #[test]
    fn test_builder_chaining() {
        let ctx = valid_builder().build().expect("build should succeed");
        assert_eq!(ctx.project.name, "test-project");
        assert_eq!(ctx.user_request, "Fix the auth bug");
        assert_eq!(ctx.system_prompt, "You are CodeBro");
        assert_eq!(ctx.workspace_file_count(), 1);
        assert_eq!(ctx.memory_count(), 1);
        assert_eq!(ctx.constraint_count(), 1);
    }

    #[test]
    fn test_builder_missing_project() {
        let result = EngineeringContextBuilder::new()
            .task(IntentPlan {
                detected_goal: "test".to_string(),
                intent_type: "General".to_string(),
                confidence: 0.5,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .build();
        assert_eq!(
            result.unwrap_err(),
            ContextBuildError::MissingProjectIdentity
        );
    }

    #[test]
    fn test_builder_missing_task() {
        let result = EngineeringContextBuilder::new()
            .project(ProjectIdentity::new("proj", "rust"))
            .build();
        assert_eq!(result.unwrap_err(), ContextBuildError::EmptyTask);
    }

    #[test]
    fn test_builder_invalid_workspace() {
        let result = EngineeringContextBuilder::new()
            .project(ProjectIdentity::new("proj", "rust"))
            .task(IntentPlan {
                detected_goal: "test".to_string(),
                intent_type: "General".to_string(),
                confidence: 0.5,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .workspace(WorkspaceContext::new(""))
            .build();
        assert_eq!(result.unwrap_err(), ContextBuildError::InvalidWorkspace);
    }

    #[test]
    fn test_builder_duplicate_fragments() {
        let result = EngineeringContextBuilder::new()
            .project(ProjectIdentity::new("proj", "rust"))
            .task(IntentPlan {
                detected_goal: "test".to_string(),
                intent_type: "General".to_string(),
                confidence: 0.5,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .context_fragment(ContextFragment {
                source: "test".to_string(),
                content: "abc".to_string(),
                relevance_score: 0.9,
            })
            .context_fragment(ContextFragment {
                source: "test".to_string(),
                content: "abc".to_string(),
                relevance_score: 0.8,
            })
            .build();
        match result {
            Err(ContextBuildError::DuplicateFragment(ref s)) => {
                assert_eq!(s, "test");
            }
            other => panic!("Expected DuplicateFragment error, got {:?}", other),
        }
    }

    #[test]
    fn test_builder_skip_validation() {
        let ctx = EngineeringContextBuilder::new()
            .with_skip_validation()
            .build()
            .expect("build should succeed with skip_validation");
        assert!(ctx.is_empty());
    }

    #[test]
    fn test_builder_empty_context() {
        let ctx = EngineeringContextBuilder::new()
            .with_skip_validation()
            .build()
            .expect("build should succeed");
        assert!(ctx.is_empty());
        assert!(ctx.statistics.is_empty());
    }

    #[test]
    fn test_builder_large_context() {
        let mut builder = valid_builder();
        for i in 0..100 {
            builder = builder.context_fragment(ContextFragment {
                source: format!("src_{}", i),
                content: format!("content for fragment {}", i),
                relevance_score: 0.5 + (i as f64 * 0.001),
            });
        }
        let ctx = builder.build().expect("build should succeed");
        assert_eq!(ctx.fragment_count(), 100);
        assert!(ctx.estimated_tokens() > 0);
    }

    #[test]
    fn test_builder_deterministic_equality() {
        let ctx1 = valid_builder().build().expect("build should succeed");
        let ctx2 = valid_builder().build().expect("build should succeed");
        // Creation times may differ, so compare the structural fields.
        assert_eq!(ctx1.project, ctx2.project);
        assert_eq!(ctx1.task, ctx2.task);
        assert_eq!(ctx1.workspace, ctx2.workspace);
        assert_eq!(ctx1.memory, ctx2.memory);
        assert_eq!(ctx1.constraints, ctx2.constraints);
        assert_eq!(ctx1.runtime, ctx2.runtime);
        assert_eq!(ctx1.active_files, ctx2.active_files);
        assert_eq!(ctx1.user_request, ctx2.user_request);
        assert_eq!(ctx1.conversation, ctx2.conversation);
        assert_eq!(ctx1.system_prompt, ctx2.system_prompt);
    }

    #[test]
    fn test_builder_active_files_dedup_and_sort() {
        let ctx = valid_builder()
            .active_file("z.rs")
            .active_file("a.rs")
            .active_file("m.rs")
            .active_file("a.rs")
            .build()
            .expect("build should succeed");
        assert_eq!(ctx.active_files, vec!["a.rs", "m.rs", "z.rs"]);
    }

    #[test]
    fn test_builder_serialization_roundtrip() {
        let ctx = valid_builder().build().expect("build should succeed");
        let json = serde_json::to_string(&ctx).expect("serialize");
        let decoded: EngineeringContext = serde_json::from_str(&json).expect("deserialize");
        assert!(ctx.equals(&decoded));
    }

    #[test]
    fn test_builder_chaining_with_all_fields() {
        let ctx = EngineeringContextBuilder::new()
            .project(ProjectIdentity::new("full-proj", "go"))
            .task(IntentPlan {
                detected_goal: "implement feature".to_string(),
                intent_type: "Modification".to_string(),
                confidence: 0.95,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .workspace(
                WorkspaceContext::new("/home/user/project")
                    .with_file(WorkspaceFile {
                        path: "main.go".to_string(),
                        language: "go".to_string(),
                        size_bytes: 1024,
                    })
                    .with_git(true)
                    .with_package_json(false)
                    .with_cargo_toml(false)
                    .with_readme(true),
            )
            .memory(
                EngineeringMemoryContext::new()
                    .with_entries(vec![
                        MemoryEntry {
                            key: "db".to_string(),
                            value: "postgres".to_string(),
                            confidence: 0.9,
                            tier: MemoryTier::Project,
                        },
                        MemoryEntry {
                            key: "cache".to_string(),
                            value: "redis".to_string(),
                            confidence: 0.85,
                            tier: MemoryTier::Session,
                        },
                    ])
                    .with_budget(2000),
            )
            .constraints(
                ConstraintContext::new()
                    .add_constraint(EngineeringConstraint {
                        description: "Use context for timeouts".to_string(),
                        category: ConstraintCategory::Performance,
                    })
                    .add_constraint(EngineeringConstraint {
                        description: "No public unauthenticated endpoints".to_string(),
                        category: ConstraintCategory::Security,
                    }),
            )
            .runtime(
                RuntimeContext::new()
                    .with_provider("openai", "gpt-4o")
                    .with_budget(3000)
                    .with_temperature(0.1)
                    .with_seed(99)
                    .with_stream(true),
            )
            .active_files(vec!["main.go".to_string(), "go.mod".to_string()])
            .user_request("Add caching layer")
            .conversation(vec![
                ConversationMessage {
                    role: "user".to_string(),
                    content: "Add caching".to_string(),
                },
                ConversationMessage {
                    role: "assistant".to_string(),
                    content: "I'll add a caching layer.".to_string(),
                },
            ])
            .system_prompt("You are an AI coding assistant")
            .build()
            .expect("build should succeed");

        assert_eq!(ctx.project.name, "full-proj");
        assert_eq!(ctx.workspace.root_path, "/home/user/project");
        assert_eq!(ctx.memory.entry_count(), 2);
        assert_eq!(ctx.constraint_count(), 2);
        assert_eq!(
            ctx.active_files,
            vec!["go.mod".to_string(), "main.go".to_string()]
        );
        assert_eq!(ctx.conversation.len(), 2);
        assert_eq!(ctx.runtime.provider_name(), Some("openai"));
        assert!(!ctx.is_empty());
    }
}
