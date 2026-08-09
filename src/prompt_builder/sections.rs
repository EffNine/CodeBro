//! Prompt section content builders.
//!
//! Each function in this module produces the content string for a
//! single section key. The builder orchestrates their assembly.

use super::template::SectionKey;

/// Build the System Identity section.
///
/// Contains the core system prompt and role definition.
pub fn build_system_identity(config_system_prompt: &str) -> String {
    if config_system_prompt.is_empty() {
        default_system_identity()
    } else {
        config_system_prompt.trim().to_string()
    }
}

fn default_system_identity() -> String {
    r#"You are CodeBro, a terminal-native engineering intelligence runtime operating inside a developer's repository.

Your capabilities:
- Read, create, and edit files in the repository
- Execute shell commands (with user awareness)
- Inspect git status and diffs
- Understand project structure, goals, and context

Working principles:
- Recommend, don't interrogate: infer obvious engineering intent from the task and repository context, and execute low-risk, clearly implied actions without unnecessary confirmation
- Reuse existing project implementations and abstractions before creating new ones
- Keep changes scoped to the task; avoid unrelated modifications
- Validate the requested outcome using the project's appropriate verification mechanism, then stop
- Prefer the smallest change that preserves architecture, correctness, and maintainability

Your constraints:
- Never expose secrets, API keys, or credentials
- Never run destructive commands without explicit user confirmation
- Request confirmation for destructive, irreversible, externally consequential, or high-impact actions
- Ask only when genuinely blocked by missing information

Your output format:
- Use clear, structured responses
- Show code blocks with proper language tags
- Explain meaningful trade-offs only when they matter
- Provide commands the user can run directly"#
        .to_string()
}

/// Build the Project Identity section from project metadata.
pub fn build_project_identity(
    project_name: &str,
    project_info: Option<&ProjectInfoLike>,
) -> String {
    match project_info {
        Some(info) => {
            let mut lines = Vec::new();
            lines.push(format!("Project: {}", info.name));
            lines.push(format!("Language: {}", info.language));
            if let Some(ref framework) = info.framework {
                lines.push(format!("Framework: {}", framework));
            }
            if let Some(ref build_system) = info.build_system {
                lines.push(format!("Build System: {}", build_system));
            }
            if let Some(ref pkg_mgr) = info.package_manager {
                lines.push(format!("Package Manager: {}", pkg_mgr));
            }
            if let Some(ref testing) = info.testing_framework {
                lines.push(format!("Testing: {}", testing));
            }
            if !info.important_files.is_empty() {
                lines.push("Important Files:".to_string());
                for f in &info.important_files {
                    lines.push(format!("  - {}", f));
                }
            }
            lines.join("\n")
        }
        None => format!("Project: {}\nLanguage: unknown", project_name),
    }
}

/// Build the Current Task section from the intent plan.
pub fn build_current_task(intent_plan: Option<&IntentPlanLike>) -> String {
    match intent_plan {
        Some(plan) => {
            let mut lines = Vec::new();
            lines.push(format!("Detected Goal: {}", plan.detected_goal));
            lines.push(format!("Intent Type: {}", plan.intent_type));
            lines.push(format!("Confidence: {:.2}", plan.confidence));
            if plan.ambiguity {
                lines.push("WARNING: Ambiguous intent detected".to_string());
                if let Some(ref reason) = plan.ambiguity_reason {
                    lines.push(format!("  Reason: {}", reason));
                }
            }
            lines.join("\n")
        }
        None => String::new(),
    }
}

/// Build the Engineering Objective section.
///
/// This is the compact, always-on goal hierarchy. It never dumps source
/// documents — it renders only the distilled objective block plus the
/// current task and its deterministic alignment.
pub fn build_engineering_objective(
    project_name: &str,
    objective: &ObjectiveLike,
    current_task: Option<&IntentPlanLike>,
    alignment: Option<GoalAlignmentLike>,
) -> String {
    if objective.is_empty() {
        return String::new();
    }

    let task_goal = current_task.map(|t| t.detected_goal.as_str()).unwrap_or("");
    let alignment_str = alignment
        .map(|a| a.as_str().to_string())
        .unwrap_or_default();

    let mut lines = Vec::new();
    if !project_name.trim().is_empty() {
        lines.push("PROJECT".to_string());
        lines.push(project_name.trim().to_string());
        lines.push(String::new());
    }
    if !objective.end_goal.trim().is_empty() {
        lines.push("END GOAL".to_string());
        lines.push(objective.end_goal.trim().to_string());
        lines.push(String::new());
    }
    if !objective.current_objective.trim().is_empty() {
        lines.push("CURRENT OBJECTIVE".to_string());
        lines.push(objective.current_objective.trim().to_string());
        lines.push(String::new());
    }
    if !objective.current_milestone.trim().is_empty() {
        lines.push("CURRENT MILESTONE".to_string());
        lines.push(objective.current_milestone.trim().to_string());
        lines.push(String::new());
    }
    if !task_goal.trim().is_empty() {
        lines.push("CURRENT TASK".to_string());
        lines.push(task_goal.trim().to_string());
        lines.push(String::new());
    }
    if !alignment_str.trim().is_empty() {
        lines.push("TASK ALIGNMENT".to_string());
        if alignment_str == "Unclear" {
            lines.push("⚠ Task alignment unclear".to_string());
        } else {
            lines.push(alignment_str);
        }
    }
    while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    lines.join("\n")
}

/// Build the Engineering Constraints section from project metadata.
pub fn build_engineering_constraints(project_info: Option<&ProjectInfoLike>) -> String {
    match project_info {
        Some(info) => {
            let mut constraints = Vec::new();
            constraints.push(format!("Language: {}", info.language));
            if let Some(ref framework) = info.framework {
                constraints.push(format!("Framework: {}", framework));
            }
            if let Some(ref build_system) = info.build_system {
                constraints.push(format!("Build: {}", build_system));
            }
            if let Some(ref testing) = info.testing_framework {
                constraints.push(format!("Testing: {}", testing));
            }
            constraints.push("Follow existing project conventions".to_string());
            constraints.push("Respect architecture boundaries".to_string());
            constraints.join("\n")
        }
        None => String::new(),
    }
}

/// Build the Relevant Context section from assembled context.
pub fn build_relevant_context(
    relevant_files: &[ContextFileLike],
    conversation: &[ConversationMsgLike],
) -> String {
    let mut parts = Vec::new();

    if !conversation.is_empty() {
        let mut conv_lines = Vec::new();
        for msg in conversation {
            conv_lines.push(format!("[{}]: {}", msg.role, msg.content));
        }
        parts.push(format!("Conversation History:\n{}", conv_lines.join("\n")));
    }

    if !relevant_files.is_empty() {
        let mut file_lines = Vec::new();
        for file in relevant_files {
            file_lines.push(format!(
                "--- {} ({}) ---\n{}",
                file.path, file.language, file.content
            ));
        }
        parts.push(format!("Relevant Files:\n{}", file_lines.join("\n")));
    }

    parts.join("\n\n")
}

/// Build the Engineering Memory section from memory resolution results.
///
/// Only injects relevant fragments; never dumps all memory.
pub fn build_engineering_memory(
    memories: &[MemoryFragment],
    context_budget_remaining: usize,
) -> String {
    if memories.is_empty() {
        return String::new();
    }

    let mut result = Vec::new();
    let mut budget = context_budget_remaining;

    for mem in memories {
        let mem_text = format!("{}: {}", mem.key, mem.value);
        let mem_tokens = mem_text.len() / 4;
        if mem_tokens > budget {
            continue;
        }
        budget -= mem_tokens;
        result.push(mem_text);
    }

    result.join("\n")
}

/// Build the Architecture Decisions section from engineering facts.
pub fn build_architecture_decisions(arch_rules: &[ArchitectureRuleLike]) -> String {
    if arch_rules.is_empty() {
        return String::new();
    }
    let mut lines = Vec::new();
    for rule in arch_rules {
        lines.push(format!("- {}", rule.description));
    }
    lines.join("\n")
}

/// Build the Workspace Facts section from engineering facts.
pub fn build_workspace_facts(fact_count: usize, diagnostics: &[DiagnosticLike]) -> String {
    if fact_count == 0 && diagnostics.is_empty() {
        return String::new();
    }
    let mut lines = Vec::new();
    lines.push(format!("Engineering facts: {} total", fact_count));
    for diag in diagnostics {
        lines.push(format!("[{}] {}", diag.severity, diag.message));
    }
    lines.join("\n")
}

/// Build the Active Files section.
pub fn build_active_files(active_paths: &[String]) -> String {
    if active_paths.is_empty() {
        return String::new();
    }
    active_paths
        .iter()
        .map(|p| format!("- {}", p))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build the User Request section.
pub fn build_user_request(request: &str) -> String {
    request.trim().to_string()
}

/// Build the Response Instructions section based on template and intent.
pub fn build_response_instructions(template: &str, intent_type: &str) -> String {
    let base = match intent_type {
        "debugging" | "Execution" => {
            "Focus on root-cause analysis. Show diagnostic steps. Provide minimal reproduction if needed."
        }
        "review" | "Question" => {
            "Provide structured review feedback. Highlight issues by severity. Suggest improvements with rationale."
        }
        "planning" | "Workflow" => {
            "Provide a step-by-step plan. Identify dependencies. Note risks and trade-offs."
        }
        "refactoring" => {
            "Preserve existing behavior. Minimize scope of changes. Suggest incremental steps."
        }
        "architecture" => {
            "Consider scalability, maintainability, and existing patterns. Explain architectural trade-offs."
        }
        "testing" => {
            "Focus on coverage, edge cases, and test structure. Follow existing test conventions."
        }
        "documentation" => {
            "Write clear, concise documentation. Include examples where helpful. Follow project doc style."
        }
        _ => {
            "Follow project conventions. Provide clear, structured output. Explain reasoning for non-obvious decisions."
        }
    };

    format!(
        "Response Instructions:\n- Template: {}\n- {}\n- Respect the project's coding conventions and architecture.",
        template, base
    )
}

// ─── Lightweight DTOs (avoid circular deps) ─────────────────────────────

#[derive(Debug, Clone)]
pub struct ProjectInfoLike {
    pub name: String,
    pub language: String,
    pub framework: Option<String>,
    pub build_system: Option<String>,
    pub package_manager: Option<String>,
    pub testing_framework: Option<String>,
    pub important_files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct IntentPlanLike {
    pub detected_goal: String,
    pub intent_type: String,
    pub confidence: f64,
    pub ambiguity: bool,
    pub ambiguity_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContextFileLike {
    pub path: String,
    pub language: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ConversationMsgLike {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct MemoryFragment {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct ArchitectureRuleLike {
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct DiagnosticLike {
    pub severity: String,
    pub message: String,
}

/// Lightweight objective DTO to avoid circular deps.
#[derive(Debug, Clone, Default)]
pub struct ObjectiveLike {
    pub end_goal: String,
    pub project_vision: String,
    pub current_objective: String,
    pub current_milestone: String,
    pub success_criteria: Vec<String>,
    pub non_goals: Vec<String>,
}

impl ObjectiveLike {
    pub fn is_empty(&self) -> bool {
        self.end_goal.trim().is_empty()
            && self.current_objective.trim().is_empty()
            && self.current_milestone.trim().is_empty()
    }
}

/// Lightweight goal-alignment DTO.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalAlignmentLike {
    Direct,
    Supporting,
    WeaklyRelated,
    Unclear,
}

impl GoalAlignmentLike {
    pub fn as_str(self) -> &'static str {
        match self {
            GoalAlignmentLike::Direct => "Direct",
            GoalAlignmentLike::Supporting => "Supporting",
            GoalAlignmentLike::WeaklyRelated => "Weakly Related",
            GoalAlignmentLike::Unclear => "Unclear",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_system_identity_empty_config() {
        let result = build_system_identity("");
        assert!(!result.is_empty());
        assert!(result.contains("CodeBro"));
    }

    #[test]
    fn test_build_system_identity_custom() {
        let custom = "Custom system prompt";
        let result = build_system_identity(custom);
        assert_eq!(result, custom);
    }

    #[test]
    fn test_default_system_identity_recommends_not_interrogates() {
        let identity = default_system_identity();
        // The interaction principle is present.
        assert!(identity.contains("Recommend, don't interrogate"));
        // The assistant-style prompts are gone: routine actions should not
        // require unnecessary confirmation.
        assert!(
            !identity.contains("Ask for clarification when requirements are ambiguous"),
            "routine low-risk actions must not require confirmation"
        );
        assert!(!identity.contains("Always explain what you are about to do before doing it"));
        // Lazy-by-default working principles are present.
        assert!(identity.contains("Reuse existing project implementations"));
        assert!(identity.contains("then stop"));
        // Consequential actions still require confirmation.
        assert!(
            identity.contains("Never run destructive commands without explicit user confirmation")
        );
    }

    #[test]
    fn test_build_project_identity_no_info() {
        let result = build_project_identity("myproj", None);
        assert!(result.contains("myproj"));
        assert!(result.contains("unknown"));
    }

    #[test]
    fn test_build_project_identity_with_info() {
        let info = ProjectInfoLike {
            name: "myproj".to_string(),
            language: "rust".to_string(),
            framework: Some("axum".to_string()),
            build_system: Some("cargo".to_string()),
            package_manager: Some("cargo".to_string()),
            testing_framework: Some("cargo test".to_string()),
            important_files: vec!["main.rs".to_string()],
        };
        let result = build_project_identity("myproj", Some(&info));
        assert!(result.contains("rust"));
        assert!(result.contains("axum"));
        assert!(result.contains("main.rs"));
    }

    #[test]
    fn test_build_current_task_no_intent() {
        let result = build_current_task(None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_current_task_with_intent() {
        let plan = IntentPlanLike {
            detected_goal: "fix bug".to_string(),
            intent_type: "Execution".to_string(),
            confidence: 0.9,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let result = build_current_task(Some(&plan));
        assert!(result.contains("fix bug"));
        assert!(result.contains("0.90"));
    }

    #[test]
    fn test_build_constraints_empty() {
        let result = build_engineering_constraints(None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_memory_empty() {
        let result = build_engineering_memory(&[], 1000);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_memory_with_budget() {
        let fragments = vec![
            MemoryFragment {
                key: "lang".to_string(),
                value: "rust".to_string(),
            },
            MemoryFragment {
                key: "framework".to_string(),
                value: "axum".to_string(),
            },
        ];
        let result = build_engineering_memory(&fragments, 100);
        assert!(!result.is_empty());
        assert!(result.contains("rust"));
    }

    #[test]
    fn test_build_memory_respects_budget() {
        let fragments = vec![
            MemoryFragment {
                key: "a".to_string(),
                value: "x".repeat(1000),
            },
            MemoryFragment {
                key: "b".to_string(),
                value: "y".to_string(),
            },
        ];
        let result = build_engineering_memory(&fragments, 50);
        assert!(!result.contains("x".repeat(1000).as_str()));
        assert!(result.contains("y"));
    }

    #[test]
    fn test_build_architecture_decisions_empty() {
        let result = build_architecture_decisions(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_active_files_empty() {
        let result = build_active_files(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_user_request() {
        let result = build_user_request("  Hello world  ");
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn test_build_response_instructions() {
        let result = build_response_instructions("engineering", "Execution");
        assert!(result.contains("engineering"));
        assert!(!result.is_empty());
    }

    #[test]
    fn test_build_engineering_objective_empty() {
        let o = ObjectiveLike::default();
        let result = build_engineering_objective("proj", &o, None, None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_engineering_objective_hierarchy() {
        let o = ObjectiveLike {
            end_goal: "Build a terminal-native runtime.".to_string(),
            project_vision: "Vision".to_string(),
            current_objective: "Maintain software projects.".to_string(),
            current_milestone: "Sprint 27.".to_string(),
            success_criteria: Vec::new(),
            non_goals: Vec::new(),
        };
        let intent = IntentPlanLike {
            detected_goal: "Implement indexed retrieval".to_string(),
            intent_type: "Execution".to_string(),
            confidence: 0.9,
            ambiguity: false,
            ambiguity_reason: None,
        };
        let result = build_engineering_objective(
            "CodeBro",
            &o,
            Some(&intent),
            Some(GoalAlignmentLike::Supporting),
        );
        assert!(result.contains("PROJECT"));
        assert!(result.contains("CodeBro"));
        assert!(result.contains("END GOAL"));
        assert!(result.contains("CURRENT OBJECTIVE"));
        assert!(result.contains("CURRENT MILESTONE"));
        assert!(result.contains("CURRENT TASK"));
        assert!(result.contains("Implement indexed retrieval"));
        assert!(result.contains("TASK ALIGNMENT"));
        assert!(result.contains("Supporting"));
    }

    #[test]
    fn test_build_engineering_objective_unclear_alignment() {
        let o = ObjectiveLike {
            end_goal: "Goal".to_string(),
            current_objective: "Objective".to_string(),
            ..ObjectiveLike::default()
        };
        let result =
            build_engineering_objective("proj", &o, None, Some(GoalAlignmentLike::Unclear));
        assert!(result.contains("⚠ Task alignment unclear"));
    }

    #[test]
    fn test_objective_like_is_empty() {
        let o = ObjectiveLike::default();
        assert!(o.is_empty());
        let filled = ObjectiveLike {
            end_goal: "g".to_string(),
            ..ObjectiveLike::default()
        };
        assert!(!filled.is_empty());
    }
}
