//! The Prompt Compiler: assembles sections into a CompiledPrompt.
//!
//! The compiler consumes typed inputs from upstream modules and produces
//! a deterministic, structured prompt ready for provider submission.

use std::time::Instant;

use super::diagnostics::PromptDiagnostics;
use super::ordering::PromptOrdering;
use super::sections::*;
use super::statistics::PromptStatistics;
use super::template::{PromptSection, PromptTemplate, SectionKey, TemplateSelection};
use serde::{Deserialize, Serialize};

/// A fully compiled prompt ready for provider submission.
///
/// Contains the rendered prompt string, statistics, and diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledPrompt {
    pub prompt: String,
    pub statistics: PromptStatistics,
    pub diagnostics: PromptDiagnostics,
    pub template_selection: TemplateSelection,
}

impl CompiledPrompt {
    pub fn is_empty(&self) -> bool {
        self.prompt.is_empty()
    }

    pub fn length(&self) -> usize {
        self.prompt.len()
    }

    pub fn estimated_tokens(&self) -> usize {
        self.statistics.estimated_tokens
    }
}

/// The engineering intelligence compiler.
///
/// Stateless and deterministic. Given the same inputs, produces the
/// same `CompiledPrompt` every time.
#[derive(Debug, Clone)]
pub struct PromptCompiler {
    ordering: PromptOrdering,
}

impl PromptCompiler {
    pub fn new() -> Self {
        PromptCompiler {
            ordering: PromptOrdering::default(),
        }
    }

    pub fn with_ordering(mut self, ordering: PromptOrdering) -> Self {
        self.ordering = ordering;
        self
    }

    /// Compile a prompt from an `EngineeringContext`.
    ///
    /// This is the canonical entry point. It extracts all required
    /// fields from the context and compiles them deterministically.
    pub fn compile_context(
        &self,
        context: &crate::engineering_context::EngineeringContext,
    ) -> CompiledPrompt {
        let start = Instant::now();

        let project_info = Some(super::sections::ProjectInfoLike {
            name: context.project.name.clone(),
            language: context.project.primary_language().to_string(),
            framework: context.project.frameworks.first().cloned(),
            build_system: context.project.build_system.clone(),
            package_manager: context.project.package_manager.clone(),
            testing_framework: context.project.testing_framework.clone(),
            important_files: context.project.important_files.clone(),
        });

        let intent_plan = context
            .task
            .as_ref()
            .map(|t| super::sections::IntentPlanLike {
                detected_goal: t.detected_goal.clone(),
                intent_type: t.intent_type.clone(),
                confidence: t.confidence,
                ambiguity: t.ambiguity,
                ambiguity_reason: t.ambiguity_reason.clone(),
            });

        let objective_like = {
            let o = &context.objective;
            super::sections::ObjectiveLike {
                end_goal: o.end_goal.clone(),
                project_vision: o.project_vision.clone(),
                current_objective: o.current_objective.clone(),
                current_milestone: o.current_milestone.clone(),
                success_criteria: o.success_criteria.clone(),
                non_goals: o.non_goals.clone(),
            }
        };

        let alignment_like = context.goal_alignment.map(|a| match a {
            crate::engineering_objective::GoalAlignment::Direct => {
                super::sections::GoalAlignmentLike::Direct
            }
            crate::engineering_objective::GoalAlignment::Supporting => {
                super::sections::GoalAlignmentLike::Supporting
            }
            crate::engineering_objective::GoalAlignment::WeaklyRelated => {
                super::sections::GoalAlignmentLike::WeaklyRelated
            }
            crate::engineering_objective::GoalAlignment::Unclear => {
                super::sections::GoalAlignmentLike::Unclear
            }
        });

        let relevant_files: Vec<super::sections::ContextFileLike> = context
            .context_fragments
            .iter()
            .map(|f| super::sections::ContextFileLike {
                path: f.source.clone(),
                language: String::new(),
                content: f.content.clone(),
            })
            .collect();

        let conversation: Vec<super::sections::ConversationMsgLike> = context
            .conversation
            .iter()
            .map(|m| super::sections::ConversationMsgLike {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let memories: Vec<super::sections::MemoryFragment> = context
            .memory
            .entries
            .iter()
            .map(|e| super::sections::MemoryFragment {
                key: e.key.clone(),
                value: e.value.clone(),
            })
            .collect();

        let arch_rules: Vec<super::sections::ArchitectureRuleLike> = context
            .constraints
            .constraints
            .iter()
            .map(|c| super::sections::ArchitectureRuleLike {
                description: c.description.clone(),
            })
            .collect();

        let diagnostics: Vec<super::sections::DiagnosticLike> = context
            .context_fragments
            .iter()
            .filter(|f| f.source == "diagnostic")
            .map(|f| super::sections::DiagnosticLike {
                severity: "info".to_string(),
                message: f.content.clone(),
            })
            .collect();

        let fact_count = context.workspace_file_count() + context.fragment_count();

        let context_budget_remaining = context.runtime.budget_tokens.saturating_sub(
            context
                .estimated_tokens()
                .saturating_sub(context.user_request.len() / 4),
        );

        let mut diag = PromptDiagnostics::new(
            select_template(intent_plan.as_ref(), project_info.as_ref()).template,
            start,
        );

        let template_selection = select_template(intent_plan.as_ref(), project_info.as_ref());
        let ordering = PromptOrdering::from_template(template_selection.template);

        let mut prompt_parts: Vec<String> = Vec::new();
        let mut section_count = 0;

        for key in &ordering.keys {
            let section = self.build_section(
                *key,
                &context.system_prompt,
                &context.project.name,
                project_info.as_ref(),
                intent_plan.as_ref(),
                &objective_like,
                alignment_like,
                &relevant_files,
                &conversation,
                &memories,
                &arch_rules,
                fact_count,
                &diagnostics,
                &context.active_files,
                &context.user_request,
                context_budget_remaining,
                &template_selection,
            );

            if section.is_empty() {
                diag.drop_section(key.as_str());
                continue;
            }

            let header = format!("=== {} ===\n", section.label);
            let content = format!("{}{}", header, section.content);
            prompt_parts.push(content);
            diag.add_section(
                &section.label,
                section.content.len() + header.len(),
                section.tokens,
            );
            section_count += 1;
        }

        let elapsed = start.elapsed();
        let prompt = prompt_parts.join("\n\n");

        let stats = PromptStatistics::new(template_selection.template, elapsed.as_nanos() as u64)
            .with_section_count(section_count)
            .with_estimated_tokens(diag.estimated_tokens)
            .with_memory_fragments(memories.len())
            .with_context_fragments(relevant_files.len());

        diag.compile_duration_ms = elapsed.as_millis() as u64;

        CompiledPrompt {
            prompt,
            statistics: stats,
            diagnostics: diag,
            template_selection,
        }
    }

    fn build_section(
        &self,
        key: SectionKey,
        system_prompt: &str,
        project_name: &str,
        project_info: Option<&ProjectInfoLike>,
        intent_plan: Option<&IntentPlanLike>,
        objective: &ObjectiveLike,
        alignment: Option<GoalAlignmentLike>,
        relevant_files: &[ContextFileLike],
        conversation: &[ConversationMsgLike],
        memories: &[MemoryFragment],
        arch_rules: &[ArchitectureRuleLike],
        fact_count: usize,
        diagnostics: &[DiagnosticLike],
        active_files: &[String],
        user_request: &str,
        context_budget_remaining: usize,
        template_selection: &TemplateSelection,
    ) -> PromptSection {
        match key {
            SectionKey::SystemIdentity => {
                let content = build_system_identity(system_prompt);
                PromptSection::new(0, "System Identity", &content)
            }
            SectionKey::ProjectIdentity => {
                let content = build_project_identity(project_name, project_info);
                PromptSection::new(1, "Project Identity", &content)
            }
            SectionKey::CurrentTask => {
                let content = build_current_task(intent_plan);
                PromptSection::new(2, "Current Task", &content)
            }
            SectionKey::EngineeringObjective => {
                let content =
                    build_engineering_objective(project_name, objective, intent_plan, alignment);
                PromptSection::new(3, "Engineering Objective", &content)
            }
            SectionKey::EngineeringConstraints => {
                let content = build_engineering_constraints(project_info);
                PromptSection::new(3, "Engineering Constraints", &content)
            }
            SectionKey::RelevantContext => {
                let content = build_relevant_context(relevant_files, conversation);
                PromptSection::new(4, "Relevant Context", &content)
            }
            SectionKey::EngineeringMemory => {
                let content = build_engineering_memory(memories, context_budget_remaining);
                PromptSection::new(5, "Engineering Memory", &content)
            }
            SectionKey::ArchitectureDecisions => {
                let content = build_architecture_decisions(arch_rules);
                PromptSection::new(6, "Architecture Decisions", &content)
            }
            SectionKey::WorkspaceFacts => {
                let content = build_workspace_facts(fact_count, diagnostics);
                PromptSection::new(7, "Workspace Facts", &content)
            }
            SectionKey::ActiveFiles => {
                let content = build_active_files(active_files);
                PromptSection::new(8, "Active Files", &content)
            }
            SectionKey::UserRequest => {
                let content = build_user_request(user_request);
                PromptSection::new(9, "User Request", &content)
            }
            SectionKey::ResponseInstructions => {
                let content = build_response_instructions(
                    template_selection.template.as_str(),
                    intent_plan
                        .map(|p| p.intent_type.as_str())
                        .unwrap_or("default"),
                );
                PromptSection::new(10, "Response Instructions", &content)
            }
        }
    }
}

/// Select the prompt template based on intent and context.
///
/// The selection is deterministic: same inputs always produce the same template.
pub fn select_template(
    intent_plan: Option<&IntentPlanLike>,
    project_info: Option<&ProjectInfoLike>,
) -> TemplateSelection {
    match intent_plan {
        Some(plan) => {
            let goal = plan.detected_goal.to_lowercase();
            let intent = plan.intent_type.to_lowercase();

            if intent == "execution" || intent == "workflow" {
                if goal.contains("debug") || goal.contains("fix") || goal.contains("error") {
                    return TemplateSelection::new(
                        PromptTemplate::Debugging,
                        "Execution intent with debug/fix/error keywords",
                    );
                }
                if goal.contains("test") {
                    return TemplateSelection::new(
                        PromptTemplate::Testing,
                        "Execution intent with test keyword",
                    );
                }
                if goal.contains("refactor") || goal.contains("restructure") {
                    return TemplateSelection::new(
                        PromptTemplate::Refactoring,
                        "Execution intent with refactor keyword",
                    );
                }
                if goal.contains("document") || goal.contains("readme") {
                    return TemplateSelection::new(
                        PromptTemplate::Documentation,
                        "Execution intent with documentation keyword",
                    );
                }
                if goal.contains("architecture") || goal.contains("design") {
                    return TemplateSelection::new(
                        PromptTemplate::Architecture,
                        "Execution intent with architecture keyword",
                    );
                }
                if goal.contains("plan") || goal.contains("design") {
                    return TemplateSelection::new(
                        PromptTemplate::Planning,
                        "Execution intent with plan/design keyword",
                    );
                }
            }

            if intent == "preference" || intent == "configuration" {
                return TemplateSelection::new(
                    PromptTemplate::Default,
                    "Preference or configuration intent",
                );
            }

            if intent == "question" {
                return TemplateSelection::new(
                    PromptTemplate::Review,
                    "Question intent — structured review format",
                );
            }

            if intent == "help" {
                return TemplateSelection::new(PromptTemplate::Default, "Help intent");
            }

            if intent == "unknown" {
                return TemplateSelection::new(
                    PromptTemplate::Default,
                    "Unknown intent — default template",
                );
            }
        }
        None => {}
    }

    TemplateSelection::new(
        PromptTemplate::Engineering,
        "Default engineering template — no specific intent match",
    )
}

impl Default for PromptCompiler {
    fn default() -> Self {
        PromptCompiler::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_project_info() -> ProjectInfoLike {
        ProjectInfoLike {
            name: "test-project".to_string(),
            language: "rust".to_string(),
            framework: Some("axum".to_string()),
            build_system: Some("cargo".to_string()),
            package_manager: Some("cargo".to_string()),
            testing_framework: Some("cargo test".to_string()),
            important_files: vec!["main.rs".to_string(), "Cargo.toml".to_string()],
        }
    }

    fn sample_intent() -> IntentPlanLike {
        IntentPlanLike {
            detected_goal: "fix authentication bug".to_string(),
            intent_type: "Execution".to_string(),
            confidence: 0.95,
            ambiguity: false,
            ambiguity_reason: None,
        }
    }

    /// Build an `EngineeringContext` from the same logical inputs the
    /// legacy parameterised `compile` accepted.
    fn build_context(
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
    ) -> crate::engineering_context::EngineeringContext {
        use crate::engineering_context::{
            builder::EngineeringContextBuilder,
            constraints::{ConstraintCategory, EngineeringConstraint},
            identity::ProjectIdentity,
            memory::{MemoryEntry, MemoryTier},
            runtime::RuntimeContext,
            workspace::{WorkspaceContext, WorkspaceFile},
            ContextFragment, ConversationMessage, EngineeringMemoryContext, IntentPlan,
        };

        let mut builder = EngineeringContextBuilder::new();

        let identity = match project_info {
            Some(info) => {
                let mut id = ProjectIdentity::new(&info.name, &info.language);
                if let Some(ref fw) = info.framework {
                    id = id.with_framework(fw);
                }
                if let Some(ref bs) = info.build_system {
                    id = id.with_build_system(bs);
                }
                if let Some(ref pm) = info.package_manager {
                    id = id.with_package_manager(pm);
                }
                if let Some(ref tf) = info.testing_framework {
                    id = id.with_testing_framework(tf);
                }
                if !info.important_files.is_empty() {
                    id = id.with_important_files(info.important_files.clone());
                }
                id
            }
            None => ProjectIdentity::new(project_name, "unknown"),
        };
        builder = builder.project(identity);

        if let Some(plan) = intent_plan {
            builder = builder.task(IntentPlan {
                detected_goal: plan.detected_goal.clone(),
                intent_type: plan.intent_type.clone(),
                confidence: plan.confidence,
                ambiguity: plan.ambiguity,
                ambiguity_reason: plan.ambiguity_reason.clone(),
            });
        } else {
            builder = builder.with_skip_validation();
        }

        let mut fragments: Vec<ContextFragment> = Vec::new();
        for file in relevant_files {
            fragments.push(ContextFragment {
                source: file.path.clone(),
                content: file.content.clone(),
                relevance_score: 0.9,
            });
        }
        for diag in diagnostics {
            fragments.push(ContextFragment {
                source: "diagnostic".to_string(),
                content: diag.message.clone(),
                relevance_score: 0.0,
            });
        }

        let mut workspace = WorkspaceContext::new(".");
        let mut pad = 0;
        while fragments.len() + pad < fact_count {
            workspace = workspace.with_file(WorkspaceFile {
                path: format!("__fact_{}.rs", pad),
                language: "rust".to_string(),
                size_bytes: 16,
            });
            pad += 1;
        }

        if !fragments.is_empty() {
            builder = builder.context_fragments(fragments);
        }
        if pad > 0 {
            builder = builder.workspace(workspace);
        }

        if !conversation.is_empty() {
            builder = builder.conversation(
                conversation
                    .iter()
                    .map(|m| ConversationMessage {
                        role: m.role.clone(),
                        content: m.content.clone(),
                    })
                    .collect(),
            );
        }

        if !memories.is_empty() {
            builder = builder.memory(
                EngineeringMemoryContext::new()
                    .with_entries(
                        memories
                            .iter()
                            .map(|m| MemoryEntry {
                                key: m.key.clone(),
                                value: m.value.clone(),
                                confidence: 0.9,
                                tier: MemoryTier::Project,
                            })
                            .collect(),
                    )
                    .with_budget(context_budget_remaining),
            );
        }

        if !arch_rules.is_empty() {
            let mut constraints = crate::engineering_context::constraints::ConstraintContext::new();
            for rule in arch_rules {
                constraints = constraints.add_constraint(EngineeringConstraint {
                    description: rule.description.clone(),
                    category: ConstraintCategory::Architecture,
                });
            }
            builder = builder.constraints(constraints);
        }

        if !active_files.is_empty() {
            builder = builder.active_files(active_files.to_vec());
        }

        if context_budget_remaining > 0 {
            builder = builder.runtime(RuntimeContext::new().with_budget(context_budget_remaining));
        }

        builder
            .user_request(user_request)
            .system_prompt(system_prompt)
            .build()
            .expect("build should succeed")
    }

    #[test]
    fn test_compile_basic() {
        let compiler = PromptCompiler::new();
        let result = compiler.compile_context(&build_context(
            "",
            "test-project",
            Some(&sample_project_info()),
            Some(&sample_intent()),
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "Fix the auth bug",
            1000,
        ));

        assert!(!result.prompt.is_empty());
        assert!(result.statistics.section_count > 0);
        assert_eq!(
            result.template_selection.template,
            PromptTemplate::Debugging
        );
    }

    #[test]
    fn test_compile_empty_inputs() {
        let compiler = PromptCompiler::new();
        let result = compiler.compile_context(&build_context(
            "",
            "empty-project",
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
        ));

        assert!(!result.prompt.is_empty());
        assert!(result.statistics.section_count >= 2);
        assert_eq!(
            result.template_selection.template,
            PromptTemplate::Engineering
        );
    }

    #[test]
    fn test_compile_deterministic() {
        let compiler = PromptCompiler::new();
        let inputs = (
            "",
            "test-project",
            Some(&sample_project_info()),
            Some(&sample_intent()),
            &[] as &[ContextFileLike],
            &[] as &[ConversationMsgLike],
            &[] as &[MemoryFragment],
            &[] as &[ArchitectureRuleLike],
            0,
            &[] as &[DiagnosticLike],
            &[] as &[String],
            "Fix the auth bug",
            1000,
        );

        let ctx1 = build_context(
            inputs.0, inputs.1, inputs.2, inputs.3, inputs.4, inputs.5, inputs.6, inputs.7,
            inputs.8, inputs.9, inputs.10, inputs.11, inputs.12,
        );
        let ctx2 = build_context(
            inputs.0, inputs.1, inputs.2, inputs.3, inputs.4, inputs.5, inputs.6, inputs.7,
            inputs.8, inputs.9, inputs.10, inputs.11, inputs.12,
        );
        let r1 = compiler.compile_context(&ctx1);
        let r2 = compiler.compile_context(&ctx2);

        assert_eq!(r1.prompt, r2.prompt);
        assert_eq!(r1.statistics.section_count, r2.statistics.section_count);
        assert_eq!(r1.template_selection, r2.template_selection);
    }

    #[test]
    fn test_template_selection_engineering() {
        let selection = select_template(None, None);
        assert_eq!(selection.template, PromptTemplate::Engineering);
    }

    #[test]
    fn test_template_selection_debugging() {
        let intent = IntentPlanLike {
            detected_goal: "fix error in auth".to_string(),
            intent_type: "Execution".to_string(),
            confidence: 0.9,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let selection = select_template(Some(&intent), None);
        assert_eq!(selection.template, PromptTemplate::Debugging);
    }

    #[test]
    fn test_template_selection_review() {
        let intent = IntentPlanLike {
            detected_goal: "what is this function?".to_string(),
            intent_type: "Question".to_string(),
            confidence: 0.8,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let selection = select_template(Some(&intent), None);
        assert_eq!(selection.template, PromptTemplate::Review);
    }

    #[test]
    fn test_diagnostics_tracks_sections() {
        let compiler = PromptCompiler::new();
        let result = compiler.compile_context(&build_context(
            "You are CodeBro",
            "proj",
            Some(&sample_project_info()),
            Some(&sample_intent()),
            &[ContextFileLike {
                path: "src/main.rs".to_string(),
                language: "rust".to_string(),
                content: "fn main() {}".to_string(),
            }],
            &[],
            &[],
            &[],
            5,
            &[DiagnosticLike {
                severity: "warning".to_string(),
                message: "unused variable".to_string(),
            }],
            &["src/main.rs".to_string()],
            "Fix it",
            1000,
        ));

        assert!(result.diagnostics.total_length > 0);
        assert!(result.diagnostics.section_sizes.len() >= 3);
        assert!(result
            .diagnostics
            .dropped_sections
            .contains(&"engineering_memory".to_string()));
    }

    #[test]
    fn test_compile_planning_template() {
        let compiler = PromptCompiler::new();
        let intent = IntentPlanLike {
            detected_goal: "plan the sprint".to_string(),
            intent_type: "Workflow".to_string(),
            confidence: 0.85,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let result = compiler.compile_context(&build_context(
            "system",
            "proj",
            None,
            Some(&intent),
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "Plan sprint",
            1000,
        ));
        assert_eq!(result.template_selection.template, PromptTemplate::Planning);
    }

    #[test]
    fn test_compile_architecture_template() {
        let compiler = PromptCompiler::new();
        let intent = IntentPlanLike {
            detected_goal: "design architecture for services".to_string(),
            intent_type: "Execution".to_string(),
            confidence: 0.9,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let result = compiler.compile_context(&build_context(
            "system",
            "proj",
            None,
            Some(&intent),
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "Design architecture",
            1000,
        ));
        assert_eq!(
            result.template_selection.template,
            PromptTemplate::Architecture
        );
    }

    #[test]
    fn test_compile_testing_template() {
        let compiler = PromptCompiler::new();
        let intent = IntentPlanLike {
            detected_goal: "write tests for auth".to_string(),
            intent_type: "Execution".to_string(),
            confidence: 0.9,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let result = compiler.compile_context(&build_context(
            "system",
            "proj",
            None,
            Some(&intent),
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "Write tests",
            1000,
        ));
        assert_eq!(result.template_selection.template, PromptTemplate::Testing);
    }

    #[test]
    fn test_compile_documentation_template() {
        let compiler = PromptCompiler::new();
        let intent = IntentPlanLike {
            detected_goal: "write documentation for API".to_string(),
            intent_type: "Execution".to_string(),
            confidence: 0.85,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let result = compiler.compile_context(&build_context(
            "system",
            "proj",
            None,
            Some(&intent),
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "Document API",
            1000,
        ));
        assert_eq!(
            result.template_selection.template,
            PromptTemplate::Documentation
        );
    }

    #[test]
    fn test_compile_prefers_canonical_order() {
        let compiler = PromptCompiler::new();
        let result = compiler.compile_context(&build_context(
            "sys",
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
            "request",
            1000,
        ));
        // System Identity should appear before User Request
        let sys_pos = result.prompt.find("System Identity").unwrap();
        let req_pos = result.prompt.find("User Request").unwrap();
        assert!(sys_pos < req_pos);
    }

    #[test]
    fn test_compile_no_hashmap_ordering() {
        // Verify determinism by compiling the same input twice
        let compiler = PromptCompiler::new();
        let ctx1 = build_context(
            "sys",
            "proj",
            Some(&sample_project_info()),
            Some(&sample_intent()),
            &[
                ContextFileLike {
                    path: "a.rs".to_string(),
                    language: "rust".to_string(),
                    content: "fn a() {}".to_string(),
                },
                ContextFileLike {
                    path: "b.rs".to_string(),
                    language: "rust".to_string(),
                    content: "fn b() {}".to_string(),
                },
            ],
            &[],
            &[],
            &[],
            0,
            &[],
            &["b.rs".to_string(), "a.rs".to_string()],
            "test",
            1000,
        );
        let ctx2 = build_context(
            "sys",
            "proj",
            Some(&sample_project_info()),
            Some(&sample_intent()),
            &[
                ContextFileLike {
                    path: "a.rs".to_string(),
                    language: "rust".to_string(),
                    content: "fn a() {}".to_string(),
                },
                ContextFileLike {
                    path: "b.rs".to_string(),
                    language: "rust".to_string(),
                    content: "fn b() {}".to_string(),
                },
            ],
            &[],
            &[],
            &[],
            0,
            &[],
            &["b.rs".to_string(), "a.rs".to_string()],
            "test",
            1000,
        );
        let result1 = compiler.compile_context(&ctx1);
        let result2 = compiler.compile_context(&ctx2);
        assert_eq!(result1.prompt, result2.prompt);
    }

    #[test]
    fn test_architecture_decisions_section() {
        let compiler = PromptCompiler::new();
        let rules = vec![
            ArchitectureRuleLike {
                description: "No raw SQL in application code".to_string(),
            },
            ArchitectureRuleLike {
                description: "All errors must be wrapped with anyhow".to_string(),
            },
        ];
        let intent = IntentPlanLike {
            detected_goal: "review architecture".to_string(),
            intent_type: "Question".to_string(),
            confidence: 0.8,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let result = compiler.compile_context(&build_context(
            "system",
            "proj",
            None,
            Some(&intent),
            &[],
            &[],
            &[],
            &rules,
            0,
            &[],
            &[],
            "test",
            1000,
        ));
        assert!(result.prompt.contains("No raw SQL"));
        assert!(result.prompt.contains("anyhow"));
    }

    #[test]
    fn test_workspace_facts_section() {
        let compiler = PromptCompiler::new();
        let diags = vec![
            DiagnosticLike {
                severity: "info".to_string(),
                message: "module not used".to_string(),
            },
            DiagnosticLike {
                severity: "error".to_string(),
                message: "type mismatch".to_string(),
            },
        ];
        let result = compiler.compile_context(&build_context(
            "system",
            "proj",
            None,
            None,
            &[],
            &[],
            &[],
            &[],
            42,
            &diags,
            &[],
            "test",
            1000,
        ));
        assert!(result.prompt.contains("42"));
        assert!(result.prompt.contains("type mismatch"));
    }

    #[test]
    fn test_conversation_context_section() {
        let compiler = PromptCompiler::new();
        let msgs = vec![
            ConversationMsgLike {
                role: "user".to_string(),
                content: "Hello".to_string(),
            },
            ConversationMsgLike {
                role: "assistant".to_string(),
                content: "Hi there!".to_string(),
            },
        ];
        let result = compiler.compile_context(&build_context(
            "system",
            "proj",
            None,
            None,
            &[],
            &msgs,
            &[],
            &[],
            0,
            &[],
            &[],
            "Next request",
            1000,
        ));
        assert!(result.prompt.contains("Hello"));
        assert!(result.prompt.contains("Hi there!"));
    }

    #[test]
    fn test_active_files_section() {
        let compiler = PromptCompiler::new();
        let intent = IntentPlanLike {
            detected_goal: "debug the crash".to_string(),
            intent_type: "Execution".to_string(),
            confidence: 0.9,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let result = compiler.compile_context(&build_context(
            "system",
            "proj",
            None,
            Some(&intent),
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[
                "src/main.rs".to_string(),
                "src/lib.rs".to_string(),
                "Cargo.toml".to_string(),
            ],
            "test",
            1000,
        ));
        assert!(result.prompt.contains("src/main.rs"));
        assert!(result.prompt.contains("src/lib.rs"));
        assert!(result.prompt.contains("Cargo.toml"));
    }

    #[test]
    fn test_intent_ambiguity_in_prompt() {
        let compiler = PromptCompiler::new();
        let intent = IntentPlanLike {
            detected_goal: "do something".to_string(),
            intent_type: "Unknown".to_string(),
            confidence: 0.2,
            ambiguity: true,
            ambiguity_reason: Some("Vague request".to_string()),
        };
        let result = compiler.compile_context(&build_context(
            "system",
            "proj",
            None,
            Some(&intent),
            &[],
            &[],
            &[],
            &[],
            0,
            &[],
            &[],
            "Do something",
            1000,
        ));
        assert!(result.prompt.contains("Ambiguous intent"));
        assert!(result.prompt.contains("Vague request"));
    }

    #[test]
    fn test_compiled_prompt_api() {
        let compiler = PromptCompiler::new();
        let result = compiler.compile_context(&build_context(
            "sys",
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
            "req",
            1000,
        ));
        assert!(!result.is_empty());
        assert!(result.length() > 0);
        assert!(result.estimated_tokens() > 0);
    }

    #[test]
    fn test_compile_context_basic() {
        use crate::engineering_context::{
            builder::EngineeringContextBuilder,
            constraints::{ConstraintCategory, EngineeringConstraint},
            identity::ProjectIdentity,
            memory::{MemoryEntry, MemoryTier},
            workspace::WorkspaceFile,
            ContextFragment, IntentPlan,
        };

        let context = EngineeringContextBuilder::new()
            .project(ProjectIdentity::new("test-project", "rust"))
            .task(IntentPlan {
                detected_goal: "fix auth bug".to_string(),
                intent_type: "Execution".to_string(),
                confidence: 0.95,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .workspace(
                crate::engineering_context::workspace::WorkspaceContext::new(".")
                    .with_file(WorkspaceFile {
                        path: "src/main.rs".to_string(),
                        language: "rust".to_string(),
                        size_bytes: 512,
                    })
                    .with_git(true),
            )
            .memory(
                crate::engineering_context::memory::EngineeringMemoryContext::new()
                    .with_entries(vec![MemoryEntry {
                        key: "language".to_string(),
                        value: "rust".to_string(),
                        confidence: 0.95,
                        tier: MemoryTier::Project,
                    }])
                    .with_budget(1000),
            )
            .constraints(
                crate::engineering_context::constraints::ConstraintContext::new().add_constraint(
                    EngineeringConstraint {
                        description: "No raw SQL".to_string(),
                        category: ConstraintCategory::Architecture,
                    },
                ),
            )
            .context_fragment(ContextFragment {
                source: "src/main.rs".to_string(),
                content: "fn main() {}".to_string(),
                relevance_score: 0.9,
            })
            .active_file("src/main.rs".to_string())
            .user_request("Fix the auth bug")
            .system_prompt("You are CodeBro")
            .build()
            .expect("build should succeed");

        let compiler = PromptCompiler::new();
        let result = compiler.compile_context(&context);

        assert!(!result.prompt.is_empty());
        assert!(result.statistics.section_count > 0);
        assert_eq!(
            result.template_selection.template,
            PromptTemplate::Debugging
        );
    }

    #[test]
    fn test_compile_context_deterministic() {
        use crate::engineering_context::{
            builder::EngineeringContextBuilder, identity::ProjectIdentity, IntentPlan,
        };

        let context1 = EngineeringContextBuilder::new()
            .project(ProjectIdentity::new("proj", "rust"))
            .task(IntentPlan {
                detected_goal: "fix bug".to_string(),
                intent_type: "Execution".to_string(),
                confidence: 0.9,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .user_request("Fix the bug")
            .system_prompt("sys")
            .build()
            .expect("build should succeed");

        let context2 = EngineeringContextBuilder::new()
            .project(ProjectIdentity::new("proj", "rust"))
            .task(IntentPlan {
                detected_goal: "fix bug".to_string(),
                intent_type: "Execution".to_string(),
                confidence: 0.9,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .user_request("Fix the bug")
            .system_prompt("sys")
            .build()
            .expect("build should succeed");

        let compiler = PromptCompiler::new();
        let r1 = compiler.compile_context(&context1);
        let r2 = compiler.compile_context(&context2);

        assert_eq!(r1.prompt, r2.prompt);
        assert_eq!(r1.statistics.section_count, r2.statistics.section_count);
        assert_eq!(r1.template_selection, r2.template_selection);
    }

    #[test]
    fn test_compile_context_empty() {
        use crate::engineering_context::{
            builder::EngineeringContextBuilder, identity::ProjectIdentity, IntentPlan,
        };

        let context = EngineeringContextBuilder::new()
            .project(ProjectIdentity::new("empty-proj", "rust"))
            .task(IntentPlan {
                detected_goal: "test".to_string(),
                intent_type: "General".to_string(),
                confidence: 0.5,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .user_request("")
            .system_prompt("sys")
            .build()
            .expect("build should succeed");

        let compiler = PromptCompiler::new();
        let result = compiler.compile_context(&context);

        assert!(!result.prompt.is_empty());
        assert!(result.statistics.section_count >= 2);
    }

    #[test]
    fn test_compile_includes_objective_section_when_present() {
        use crate::engineering_context::{
            builder::EngineeringContextBuilder, identity::ProjectIdentity, IntentPlan,
        };
        use crate::engineering_objective::EngineeringObjective;

        let objective = EngineeringObjective::new(
            "Build a terminal-native runtime.",
            "Vision",
            "Maintain software projects.",
            "Sprint 27.",
        );

        let context = EngineeringContextBuilder::new()
            .project(ProjectIdentity::new("codebro", "rust"))
            .task(IntentPlan {
                detected_goal: "implement indexed retrieval".to_string(),
                intent_type: "Execution".to_string(),
                confidence: 0.9,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .objective(objective)
            .user_request("implement indexed retrieval")
            .system_prompt("sys")
            .build()
            .expect("build should succeed");

        let compiler = PromptCompiler::new();
        let result = compiler.compile_context(&context);

        assert!(result.prompt.contains("Engineering Objective"));
        assert!(result.prompt.contains("END GOAL"));
        assert!(result.prompt.contains("CURRENT OBJECTIVE"));
        assert!(result.prompt.contains("CURRENT TASK"));
    }

    #[test]
    fn test_compile_drops_objective_section_when_empty() {
        use crate::engineering_context::{
            builder::EngineeringContextBuilder, identity::ProjectIdentity, IntentPlan,
        };

        let context = EngineeringContextBuilder::new()
            .project(ProjectIdentity::new("proj", "rust"))
            .task(IntentPlan {
                detected_goal: "fix bug".to_string(),
                intent_type: "Execution".to_string(),
                confidence: 0.9,
                ambiguity: false,
                ambiguity_reason: None,
            })
            .user_request("fix the bug")
            .system_prompt("sys")
            .build()
            .expect("build should succeed");

        let compiler = PromptCompiler::new();
        let result = compiler.compile_context(&context);

        assert!(!result.prompt.contains("Engineering Objective"));
    }
}
