//! Prompt Builder v2 — Public API.
//!
//! The `PromptBuilder` is the entry point for compiling engineering
//! context into an optimal prompt. It wraps the `PromptCompiler` and
//! accepts typed inputs from upstream modules.

use super::compiler::{CompiledPrompt, PromptCompiler};
use super::ordering::PromptOrdering;
use super::template::{PromptTemplate, SectionKey, TemplateSelection};

use super::sections::{
    ArchitectureRuleLike, ContextFileLike, ConversationMsgLike, DiagnosticLike, IntentPlanLike,
    MemoryFragment, ProjectInfoLike,
};

/// The Prompt Builder — compiles engineering context into a prompt.
///
/// Consumes:
/// - ContextAssemblyResult (mapped to internal DTOs)
/// - Project Identity
/// - Engineering Memory
/// - Runtime State
/// - Active Task (IntentPlan)
/// - Provider Metadata
///
/// Produces:
/// - CompiledPrompt
#[derive(Debug, Clone)]
pub struct PromptBuilder {
    compiler: PromptCompiler,
    default_template: PromptTemplate,
}

impl PromptBuilder {
    /// Create a new PromptBuilder with default ordering.
    pub fn new() -> Self {
        PromptBuilder {
            compiler: PromptCompiler::new(),
            default_template: PromptTemplate::Engineering,
        }
    }

    /// Create a PromptBuilder with custom section ordering.
    pub fn with_ordering(ordering: PromptOrdering) -> Self {
        PromptBuilder {
            compiler: PromptCompiler::new().with_ordering(ordering),
            default_template: PromptTemplate::Engineering,
        }
    }

    /// Set the default template when no intent match is found.
    pub fn with_default_template(mut self, template: PromptTemplate) -> Self {
        self.default_template = template;
        self
    }

    /// Compile a prompt from all available context sources.
    ///
    /// This is the main entry point. All inputs are optional; the
    /// compiler handles missing data gracefully.
    ///
    /// # Arguments
    /// * `system_prompt` — system identity content (from ContextConfig or default)
    /// * `project_name` — project name
    /// * `project_info` — optional project metadata from scanner
    /// * `intent_plan` — optional intent classification from intent engine
    /// * `relevant_files` — assembled context files from context builder
    /// * `conversation` — conversation history messages
    /// * `memories` — engineering memory fragments from memory runtime
    /// * `arch_rules` — architecture decision facts from engineering facts
    /// * `fact_count` — total engineering fact count
    /// * `diagnostics` — engineering diagnostics
    /// * `active_files` — currently active file paths
    /// * `user_request` — the raw user request
    /// * `context_budget_remaining` — token budget remaining after other sections
    pub fn compile(
        &self,
        system_prompt: &str,
        project_name: &str,
        project_info: Option<&ProjectInfoLike>,
        intent_plan: Option<&IntentPlanLike>,
        relevant_files: &[ContextFileLike],
        conversation: &[ConversationMsgLike],
        memories: &[MemoryFragment],
        arch_rules: &[ArchitectureRuleLike],
        fact_count: usize,
        diagnostics: &[DiagnosticLike],
        active_files: &[String],
        user_request: &str,
        context_budget_remaining: usize,
    ) -> CompiledPrompt {
        self.compiler.compile(
            system_prompt,
            project_name,
            project_info,
            intent_plan,
            relevant_files,
            conversation,
            memories,
            arch_rules,
            fact_count,
            diagnostics,
            active_files,
            user_request,
            context_budget_remaining,
        )
    }

    /// Compile using the default template for selection.
    pub fn compile_with_default_template(
        &self,
        system_prompt: &str,
        project_name: &str,
        project_info: Option<&ProjectInfoLike>,
        intent_plan: Option<&IntentPlanLike>,
        relevant_files: &[ContextFileLike],
        conversation: &[ConversationMsgLike],
        memories: &[MemoryFragment],
        arch_rules: &[ArchitectureRuleLike],
        fact_count: usize,
        diagnostics: &[DiagnosticLike],
        active_files: &[String],
        user_request: &str,
        context_budget_remaining: usize,
    ) -> CompiledPrompt {
        let mut builder = self.clone();
        builder.default_template = self.default_template;
        builder.compile(
            system_prompt,
            project_name,
            project_info,
            intent_plan,
            relevant_files,
            conversation,
            memories,
            arch_rules,
            fact_count,
            diagnostics,
            active_files,
            user_request,
            context_budget_remaining,
        )
    }

    /// Get the selected template for a given intent (without full compilation).
    pub fn select_template(
        &self,
        intent_plan: Option<&IntentPlanLike>,
        project_info: Option<&ProjectInfoLike>,
    ) -> TemplateSelection {
        super::compiler::select_template(intent_plan, project_info)
    }

    /// Returns the default template.
    pub fn default_template(&self) -> PromptTemplate {
        self.default_template
    }
}

impl Default for PromptBuilder {
    fn default() -> Self {
        PromptBuilder::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_creation() {
        let builder = PromptBuilder::new();
        assert_eq!(builder.default_template(), PromptTemplate::Engineering);
    }

    #[test]
    fn test_builder_with_ordering() {
        let ordering =
            PromptOrdering::from_keys(vec![SectionKey::SystemIdentity, SectionKey::UserRequest]);
        let builder = PromptBuilder::with_ordering(ordering);
        // Compiler has custom ordering; default should still work
        let result = builder.compile(
            "system",
            "proj",
            None,
            None,
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "hello",
            1000,
        );
        assert!(!result.prompt.is_empty());
    }

    #[test]
    fn test_builder_empty_compile() {
        let builder = PromptBuilder::new();
        let result = builder.compile(
            "",
            "my-project",
            None,
            None,
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "",
            1000,
        );
        assert!(!result.prompt.is_empty());
        assert!(result.statistics.section_count > 0);
    }

    #[test]
    fn test_builder_deterministic() {
        let builder = PromptBuilder::new();
        let inputs = (
            "system prompt",
            "proj",
            None::<&ProjectInfoLike>,
            None::<&IntentPlanLike>,
            &[] as &[ContextFileLike],
            &[] as &[ConversationMsgLike],
            &[] as &[MemoryFragment],
            &[] as &[ArchitectureRuleLike],
            0,
            &[] as &[DiagnosticLike],
            &[] as &[String],
            "user request",
            500,
        );

        let r1 = builder.compile(
            inputs.0, inputs.1, inputs.2, inputs.3, inputs.4, inputs.5, inputs.6, inputs.7,
            inputs.8, inputs.9, inputs.10, inputs.11, inputs.12,
        );
        let r2 = builder.compile(
            inputs.0, inputs.1, inputs.2, inputs.3, inputs.4, inputs.5, inputs.6, inputs.7,
            inputs.8, inputs.9, inputs.10, inputs.11, inputs.12,
        );

        assert_eq!(r1.prompt, r2.prompt);
    }

    #[test]
    fn test_builder_template_selection() {
        let builder = PromptBuilder::new();
        let intent = IntentPlanLike {
            detected_goal: "debug the crash".to_string(),
            intent_type: "Execution".to_string(),
            confidence: 0.9,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let selection = builder.select_template(Some(&intent), None);
        assert_eq!(selection.template, PromptTemplate::Debugging);
    }

    #[test]
    fn test_builder_engineering_template() {
        let builder = PromptBuilder::new();
        let intent = IntentPlanLike {
            detected_goal: "implement new feature".to_string(),
            intent_type: "Execution".to_string(),
            confidence: 0.95,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let result = builder.compile(
            "system",
            "myproj",
            None,
            Some(&intent),
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "Add auth module",
            1000,
        );
        assert_eq!(
            result.template_selection.template,
            PromptTemplate::Engineering
        );
    }

    #[test]
    fn test_builder_review_template() {
        let builder = PromptBuilder::new();
        let intent = IntentPlanLike {
            detected_goal: "review the code".to_string(),
            intent_type: "Question".to_string(),
            confidence: 0.8,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let result = builder.compile(
            "system",
            "myproj",
            None,
            Some(&intent),
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "Review auth module",
            1000,
        );
        assert_eq!(result.template_selection.template, PromptTemplate::Review);
    }

    #[test]
    fn test_builder_debug_template() {
        let builder = PromptBuilder::new();
        let intent = IntentPlanLike {
            detected_goal: "fix login bug".to_string(),
            intent_type: "Execution".to_string(),
            confidence: 0.9,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let result = builder.compile(
            "system",
            "myproj",
            None,
            Some(&intent),
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "Fix login bug",
            1000,
        );
        assert_eq!(
            result.template_selection.template,
            PromptTemplate::Debugging
        );
    }

    #[test]
    fn test_builder_refactor_template() {
        let builder = PromptBuilder::new();
        let intent = IntentPlanLike {
            detected_goal: "refactor auth module".to_string(),
            intent_type: "Execution".to_string(),
            confidence: 0.85,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let result = builder.compile(
            "system",
            "myproj",
            None,
            Some(&intent),
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "Refactor auth",
            1000,
        );
        assert_eq!(
            result.template_selection.template,
            PromptTemplate::Refactoring
        );
    }

    #[test]
    fn test_builder_memory_injection() {
        let builder = PromptBuilder::new();
        let memories = vec![
            MemoryFragment {
                key: "lang".to_string(),
                value: "rust".to_string(),
            },
            MemoryFragment {
                key: "framework".to_string(),
                value: "axum".to_string(),
            },
            MemoryFragment {
                key: "style".to_string(),
                value: "no clippy warnings".to_string(),
            },
        ];
        let result = builder.compile(
            "system",
            "myproj",
            None,
            None,
            &[],
            &[],
            &memories,
            &[],
            0,
            &[],
            &[],
            "Hello",
            500,
        );
        assert!(result.prompt.contains("rust"));
        assert!(result.prompt.contains("axum"));
        assert!(result.prompt.contains("no clippy warnings"));
    }

    #[test]
    fn test_builder_constraint_injection() {
        let builder = PromptBuilder::new();
        let info = ProjectInfoLike {
            name: "myproj".to_string(),
            language: "rust".to_string(),
            framework: Some("axum".to_string()),
            build_system: Some("cargo".to_string()),
            package_manager: None,
            testing_framework: Some("cargo test".to_string()),
            important_files: vec!["main.rs".to_string()],
        };
        let result = builder.compile(
            "system",
            "myproj",
            Some(&info),
            None,
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "Hello",
            500,
        );
        assert!(result.prompt.contains("axum"));
        assert!(result.prompt.contains("cargo"));
        assert!(result.prompt.contains("rust"));
    }

    #[test]
    fn test_builder_project_identity() {
        let builder = PromptBuilder::new();
        let info = ProjectInfoLike {
            name: "codebro".to_string(),
            language: "rust".to_string(),
            framework: Some("actix-web".to_string()),
            build_system: Some("cargo".to_string()),
            package_manager: Some("cargo".to_string()),
            testing_framework: None,
            important_files: vec!["main.rs".to_string(), "Cargo.toml".to_string()],
        };
        let result = builder.compile(
            "system",
            "codebro",
            Some(&info),
            None,
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "Test",
            500,
        );
        assert!(result.prompt.contains("codebro"));
        assert!(result.prompt.contains("actix-web"));
        assert!(result.prompt.contains("main.rs"));
    }

    #[test]
    fn test_builder_large_context() {
        let builder = PromptBuilder::new();
        let files: Vec<ContextFileLike> = (0..50)
            .map(|i| ContextFileLike {
                path: format!("src/module_{}.rs", i),
                language: "rust".to_string(),
                content: format!("fn main_{}() {{}}", i),
            })
            .collect();
        let conversations: Vec<ConversationMsgLike> = (0..20)
            .map(|i| ConversationMsgLike {
                role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                content: format!("Message number {}", i),
            })
            .collect();
        let result = builder.compile(
            "system",
            "large-proj",
            None,
            None,
            &files,
            &conversations,
            &[],
            &[],
            100,
            &[],
            &[],
            "Process large context",
            5000,
        );
        assert!(!result.prompt.is_empty());
        assert!(result.statistics.section_count > 0);
    }

    #[test]
    fn test_builder_statistics_exposed() {
        let builder = PromptBuilder::new();
        let result = builder.compile(
            "system",
            "proj",
            None,
            None,
            &[ContextFileLike {
                path: "src/lib.rs".to_string(),
                language: "rust".to_string(),
                content: "pub fn hello() {}".to_string(),
            }],
            &[ConversationMsgLike {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            &[MemoryFragment {
                key: "lang".to_string(),
                value: "rust".to_string(),
            }],
            &[],
            10,
            &[],
            &["src/lib.rs".to_string()],
            "hello world",
            1000,
        );
        assert!(result.statistics.section_count > 0);
        assert!(result.statistics.estimated_tokens > 0);
        assert_eq!(result.statistics.template, "engineering");
        assert!(result.statistics.compile_time_ns > 0);
        assert_eq!(result.statistics.memory_fragments, 1);
        assert_eq!(result.statistics.context_fragments, 1);
    }

    #[test]
    fn test_builder_diagnostics_exposed() {
        let builder = PromptBuilder::new();
        let result = builder.compile(
            "system prompt",
            "proj",
            None,
            None,
            &[],
            &[],
            &[],
            &[],
            5,
            &[DiagnosticLike {
                severity: "error".to_string(),
                message: "dead code".to_string(),
            }],
            &[],
            "user request",
            500,
        );
        assert!(result.diagnostics.total_length > 0);
        assert_eq!(result.diagnostics.template_used, "engineering");
        assert!(result.diagnostics.compile_duration_ms >= 0);
    }
}
