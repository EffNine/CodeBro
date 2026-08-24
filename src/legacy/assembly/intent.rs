use serde::{Deserialize, Serialize};

/// Classification of a user request produced by the intent pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntentType {
    /// Request is about understanding the codebase (symbols, structure).
    Understanding,
    /// Request is about modifying or creating code.
    Modification,
    /// Request is about debugging or fixing issues.
    Debugging,
    /// Request is about project-level knowledge (memory, facts).
    ProjectKnowledge,
    /// Request is about the current workspace state (git, files).
    WorkspaceState,
    /// Request cannot be classified; defaults to broad context.
    General,
}

impl std::fmt::Display for IntentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntentType::Understanding => write!(f, "understanding"),
            IntentType::Modification => write!(f, "modification"),
            IntentType::Debugging => write!(f, "debugging"),
            IntentType::ProjectKnowledge => write!(f, "project_knowledge"),
            IntentType::WorkspaceState => write!(f, "workspace_state"),
            IntentType::General => write!(f, "general"),
        }
    }
}

/// Result of the intent-classification step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentClassification {
    pub intent: IntentType,
    /// Keywords extracted from the request for use by source selectors.
    pub keywords: Vec<String>,
    /// Whether diagnostics should be prioritised.
    pub prioritise_diagnostics: bool,
}

impl IntentClassification {
    pub fn classify(request: &str) -> Self {
        let lower = request.to_lowercase();
        let keywords: Vec<String> = lower
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .map(|s| s.to_string())
            .collect();

        let intent = if lower.contains("fix")
            || lower.contains("bug")
            || lower.contains("error")
            || lower.contains("why does")
            || lower.contains("not working")
        {
            IntentType::Debugging
        } else if lower.contains("add")
            || lower.contains("create")
            || lower.contains("implement")
            || lower.contains("change")
            || lower.contains("modify")
            || lower.contains("edit")
            || lower.contains("refactor")
        {
            IntentType::Modification
        } else if lower.contains("how does")
            || lower.contains("what is")
            || lower.contains("explain")
            || lower.contains("understand")
            || lower.contains("describe")
        {
            IntentType::Understanding
        } else if lower.contains("memory")
            || lower.contains("remember")
            || lower.contains("project")
            || lower.contains("architecture")
        {
            IntentType::ProjectKnowledge
        } else if lower.contains("git")
            || lower.contains("branch")
            || lower.contains("diff")
            || lower.contains("status")
            || lower.contains("changed")
        {
            IntentType::WorkspaceState
        } else {
            IntentType::General
        };

        let prioritise_diagnostics = matches!(intent, IntentType::Debugging);

        IntentClassification {
            intent,
            keywords,
            prioritise_diagnostics,
        }
    }

    /// Suggest which sources to query for this intent.
    pub fn source_preferences(&self) -> Vec<&'static str> {
        match self.intent {
            IntentType::Debugging => vec![
                "engineering_facts",
                "scanner",
                "git",
                "indexer",
                "workspace",
                "memory",
            ],
            IntentType::Modification => {
                vec!["indexer", "engineering_facts", "workspace", "git", "memory"]
            }
            IntentType::Understanding => vec![
                "engineering_facts",
                "indexer",
                "scanner",
                "workspace",
                "memory",
            ],
            IntentType::ProjectKnowledge => {
                vec!["memory", "engineering_facts", "scanner", "workspace"]
            }
            IntentType::WorkspaceState => vec!["git", "workspace", "scanner", "memory"],
            IntentType::General => vec![
                "workspace",
                "indexer",
                "engineering_facts",
                "memory",
                "git",
                "scanner",
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_debugging() {
        let c = IntentClassification::classify("fix the bug in auth");
        assert_eq!(c.intent, IntentType::Debugging);
        assert!(c.prioritise_diagnostics);
    }

    #[test]
    fn test_classify_modification() {
        let c = IntentClassification::classify("add a new endpoint");
        assert_eq!(c.intent, IntentType::Modification);
    }

    #[test]
    fn test_classify_understanding() {
        let c = IntentClassification::classify("explain how the parser works");
        assert_eq!(c.intent, IntentType::Understanding);
    }

    #[test]
    fn test_classify_workspace() {
        let c = IntentClassification::classify("git status branch diff");
        assert_eq!(c.intent, IntentType::WorkspaceState);
    }

    #[test]
    fn test_classify_general() {
        let c = IntentClassification::classify("hello");
        assert_eq!(c.intent, IntentType::General);
    }

    #[test]
    fn test_source_preferences_debugging() {
        let c = IntentClassification::classify("fix the auth bug");
        let prefs = c.source_preferences();
        assert!(prefs.contains(&"engineering_facts"));
        assert!(prefs.contains(&"scanner"));
    }
}
