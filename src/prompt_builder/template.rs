//! Core types for the Prompt Builder v2.
//!
//! Defines `PromptSection`, `PromptTemplate`, `TemplateSelection`,
//! and supporting enums used across the compiler pipeline.

use serde::{Deserialize, Serialize};

/// A single ordered section of a compiled prompt.
///
/// Each section carries its position, label, and raw content.
/// Sections are always emitted in deterministic order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSection {
    pub order: usize,
    pub label: String,
    pub content: String,
    pub tokens: usize,
}

impl PromptSection {
    pub fn new(order: usize, label: &str, content: &str) -> Self {
        let tokens = estimate_tokens(content);
        PromptSection {
            order,
            label: label.to_string(),
            content: content.to_string(),
            tokens,
        }
    }

    pub fn empty(order: usize, label: &str) -> Self {
        PromptSection {
            order,
            label: label.to_string(),
            content: String::new(),
            tokens: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }
}

/// The prompt template selected for a given intent and context.
///
/// Templates define section order, inclusion rules, and
/// contextual framing for different task types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PromptTemplate {
    Engineering,
    Debugging,
    Review,
    Planning,
    Refactoring,
    Architecture,
    Testing,
    Documentation,
    Default,
}

impl PromptTemplate {
    pub fn as_str(self) -> &'static str {
        match self {
            PromptTemplate::Engineering => "engineering",
            PromptTemplate::Debugging => "debugging",
            PromptTemplate::Review => "review",
            PromptTemplate::Planning => "planning",
            PromptTemplate::Refactoring => "refactoring",
            PromptTemplate::Architecture => "architecture",
            PromptTemplate::Testing => "testing",
            PromptTemplate::Documentation => "documentation",
            PromptTemplate::Default => "default",
        }
    }

    pub fn section_order(self) -> &'static [SectionKey] {
        match self {
            PromptTemplate::Engineering
            | PromptTemplate::Default
            | PromptTemplate::Architecture => &[
                SectionKey::SystemIdentity,
                SectionKey::ProjectIdentity,
                SectionKey::CurrentTask,
                SectionKey::EngineeringObjective,
                SectionKey::EngineeringConstraints,
                SectionKey::RelevantContext,
                SectionKey::EngineeringMemory,
                SectionKey::WorkspaceFacts,
                SectionKey::UserRequest,
                SectionKey::ResponseInstructions,
            ],
            PromptTemplate::Debugging => &[
                SectionKey::SystemIdentity,
                SectionKey::ProjectIdentity,
                SectionKey::CurrentTask,
                SectionKey::EngineeringObjective,
                SectionKey::EngineeringConstraints,
                SectionKey::ActiveFiles,
                SectionKey::RelevantContext,
                SectionKey::EngineeringMemory,
                SectionKey::UserRequest,
                SectionKey::ResponseInstructions,
            ],
            PromptTemplate::Review => &[
                SectionKey::SystemIdentity,
                SectionKey::ProjectIdentity,
                SectionKey::CurrentTask,
                SectionKey::EngineeringObjective,
                SectionKey::ArchitectureDecisions,
                SectionKey::RelevantContext,
                SectionKey::EngineeringMemory,
                SectionKey::ActiveFiles,
                SectionKey::UserRequest,
                SectionKey::ResponseInstructions,
            ],
            PromptTemplate::Planning => &[
                SectionKey::SystemIdentity,
                SectionKey::ProjectIdentity,
                SectionKey::CurrentTask,
                SectionKey::EngineeringObjective,
                SectionKey::EngineeringConstraints,
                SectionKey::RelevantContext,
                SectionKey::EngineeringMemory,
                SectionKey::UserRequest,
                SectionKey::ResponseInstructions,
            ],
            PromptTemplate::Refactoring => &[
                SectionKey::SystemIdentity,
                SectionKey::ProjectIdentity,
                SectionKey::CurrentTask,
                SectionKey::EngineeringObjective,
                SectionKey::ArchitectureDecisions,
                SectionKey::RelevantContext,
                SectionKey::EngineeringMemory,
                SectionKey::ActiveFiles,
                SectionKey::UserRequest,
                SectionKey::ResponseInstructions,
            ],
            PromptTemplate::Testing => &[
                SectionKey::SystemIdentity,
                SectionKey::ProjectIdentity,
                SectionKey::CurrentTask,
                SectionKey::EngineeringObjective,
                SectionKey::EngineeringConstraints,
                SectionKey::RelevantContext,
                SectionKey::EngineeringMemory,
                SectionKey::ActiveFiles,
                SectionKey::UserRequest,
                SectionKey::ResponseInstructions,
            ],
            PromptTemplate::Documentation => &[
                SectionKey::SystemIdentity,
                SectionKey::ProjectIdentity,
                SectionKey::CurrentTask,
                SectionKey::EngineeringObjective,
                SectionKey::RelevantContext,
                SectionKey::EngineeringMemory,
                SectionKey::UserRequest,
                SectionKey::ResponseInstructions,
            ],
        }
    }
}

/// Stable key identifying a prompt section.
///
/// Used to look up content during compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SectionKey {
    SystemIdentity,
    ProjectIdentity,
    CurrentTask,
    EngineeringObjective,
    EngineeringConstraints,
    RelevantContext,
    EngineeringMemory,
    ArchitectureDecisions,
    WorkspaceFacts,
    ActiveFiles,
    UserRequest,
    ResponseInstructions,
}

impl SectionKey {
    pub fn as_str(self) -> &'static str {
        match self {
            SectionKey::SystemIdentity => "system_identity",
            SectionKey::ProjectIdentity => "project_identity",
            SectionKey::CurrentTask => "current_task",
            SectionKey::EngineeringObjective => "engineering_objective",
            SectionKey::EngineeringConstraints => "engineering_constraints",
            SectionKey::RelevantContext => "relevant_context",
            SectionKey::EngineeringMemory => "engineering_memory",
            SectionKey::ArchitectureDecisions => "architecture_decisions",
            SectionKey::WorkspaceFacts => "workspace_facts",
            SectionKey::ActiveFiles => "active_files",
            SectionKey::UserRequest => "user_request",
            SectionKey::ResponseInstructions => "response_instructions",
        }
    }
}

/// The result of template selection.
///
/// Carries the selected template and the reasoning behind the choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateSelection {
    pub template: PromptTemplate,
    pub reasoning: String,
}

impl TemplateSelection {
    pub fn new(template: PromptTemplate, reasoning: &str) -> Self {
        TemplateSelection {
            template,
            reasoning: reasoning.to_string(),
        }
    }
}

fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_section_creation() {
        let section = PromptSection::new(1, "Test Section", "Hello world");
        assert_eq!(section.order, 1);
        assert_eq!(section.label, "Test Section");
        assert_eq!(section.tokens, 2);
    }

    #[test]
    fn test_empty_section() {
        let section = PromptSection::empty(0, "Empty");
        assert!(section.is_empty());
        assert_eq!(section.tokens, 0);
    }

    #[test]
    fn test_template_section_order() {
        let order = PromptTemplate::Engineering.section_order();
        assert!(!order.is_empty());
        assert_eq!(order[0], SectionKey::SystemIdentity);
        assert_eq!(order[order.len() - 1], SectionKey::ResponseInstructions);
    }

    #[test]
    fn test_all_templates_have_valid_order() {
        for template in [
            PromptTemplate::Engineering,
            PromptTemplate::Debugging,
            PromptTemplate::Review,
            PromptTemplate::Planning,
            PromptTemplate::Refactoring,
            PromptTemplate::Architecture,
            PromptTemplate::Testing,
            PromptTemplate::Documentation,
            PromptTemplate::Default,
        ] {
            let order = template.section_order();
            assert!(
                order.first() == Some(&SectionKey::SystemIdentity),
                "Template {:?} must start with SystemIdentity",
                template
            );
            assert!(
                order.last() == Some(&SectionKey::ResponseInstructions),
                "Template {:?} must end with ResponseInstructions",
                template
            );
        }
    }

    #[test]
    fn test_section_key_str() {
        assert_eq!(SectionKey::SystemIdentity.as_str(), "system_identity");
        assert_eq!(SectionKey::UserRequest.as_str(), "user_request");
    }

    #[test]
    fn test_template_as_str() {
        assert_eq!(PromptTemplate::Engineering.as_str(), "engineering");
        assert_eq!(PromptTemplate::Debugging.as_str(), "debugging");
        assert_eq!(PromptTemplate::Default.as_str(), "default");
    }
}
