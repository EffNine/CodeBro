#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::dispatcher::ToolDispatcher;

#[derive(Debug, Clone)]
pub struct ToolSelection {
    pub primary_tool: String,
    pub supporting_tools: Vec<String>,
    pub reasoning: String,
    pub confidence: f32,
}

pub struct SmartToolRouter {
    dispatcher: ToolDispatcher,
}

impl SmartToolRouter {
    pub fn new(dispatcher: ToolDispatcher) -> Self {
        SmartToolRouter { dispatcher }
    }

    pub fn route(&self, task: &str, context: &str) -> ToolSelection {
        let task_lower = task.to_lowercase();
        let context_lower = context.to_lowercase();

        if task_lower.contains("git")
            || task_lower.contains("diff")
            || task_lower.contains("commit")
        {
            return ToolSelection {
                primary_tool: "git_status".to_string(),
                supporting_tools: vec!["git_diff".to_string()],
                reasoning: "Git operation required".to_string(),
                confidence: 0.9,
            };
        }

        if task_lower.contains("find")
            || task_lower.contains("search")
            || task_lower.contains("where")
        {
            if context_lower.contains("auth")
                || context_lower.contains("login")
                || context_lower.contains("user")
            {
                return ToolSelection {
                    primary_tool: "semantic_search".to_string(),
                    supporting_tools: vec![
                        "symbol_lookup".to_string(),
                        "dependency_analysis".to_string(),
                    ],
                    reasoning: "Semantic search needed for code discovery".to_string(),
                    confidence: 0.9,
                };
            }
            return ToolSelection {
                primary_tool: "semantic_search".to_string(),
                supporting_tools: vec!["list_files".to_string()],
                reasoning: "Search tool required".to_string(),
                confidence: 0.8,
            };
        }

        if task_lower.contains("read")
            || task_lower.contains("show")
            || task_lower.contains("explain")
        {
            return ToolSelection {
                primary_tool: "read_file".to_string(),
                supporting_tools: vec!["list_files".to_string()],
                reasoning: "File reading required".to_string(),
                confidence: 0.9,
            };
        }

        if task_lower.contains("run")
            || task_lower.contains("test")
            || task_lower.contains("build")
            || task_lower.contains("execute")
        {
            return ToolSelection {
                primary_tool: "run_command".to_string(),
                supporting_tools: vec!["read_file".to_string()],
                reasoning: "Command execution required".to_string(),
                confidence: 0.9,
            };
        }

        if task_lower.contains("edit")
            || task_lower.contains("modify")
            || task_lower.contains("update")
            || task_lower.contains("change")
        {
            return ToolSelection {
                primary_tool: "edit_file".to_string(),
                supporting_tools: vec!["read_file".to_string(), "patch".to_string()],
                reasoning: "File modification required".to_string(),
                confidence: 0.85,
            };
        }

        if task_lower.contains("create") || task_lower.contains("add") || task_lower.contains("new")
        {
            return ToolSelection {
                primary_tool: "create_file".to_string(),
                supporting_tools: vec!["list_files".to_string()],
                reasoning: "File creation required".to_string(),
                confidence: 0.85,
            };
        }

        if task_lower.contains("list")
            || task_lower.contains("show files")
            || task_lower.contains("directory")
        {
            return ToolSelection {
                primary_tool: "list_files".to_string(),
                supporting_tools: vec![],
                reasoning: "Directory listing required".to_string(),
                confidence: 0.9,
            };
        }

        ToolSelection {
            primary_tool: "read_file".to_string(),
            supporting_tools: vec!["list_files".to_string()],
            reasoning: "Default to file reading for context".to_string(),
            confidence: 0.5,
        }
    }

    pub fn get_available_tools(&self) -> Vec<String> {
        self.dispatcher.list_tools()
    }
}
