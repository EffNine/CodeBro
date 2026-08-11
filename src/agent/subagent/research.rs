#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::agent::grounding::truncate_for_summary;
use crate::agent::subagent::{SubAgent, SubAgentCapability, SubAgentContext, SubAgentResult};

pub struct ResearchAgent;

impl ResearchAgent {
    pub fn new() -> Self {
        ResearchAgent
    }

    fn semantic_search(&self, context: &SubAgentContext) -> Vec<String> {
        context.related_symbols.clone()
    }

    fn dependency_analysis(&self, context: &SubAgentContext) -> Vec<String> {
        context.dependencies.clone()
    }

    fn file_inspection(&self, context: &SubAgentContext) -> Vec<String> {
        context.relevant_files.clone()
    }
}

impl SubAgent for ResearchAgent {
    fn name(&self) -> &str {
        "research"
    }

    fn purpose(&self) -> &str {
        "Understand codebase and gather information"
    }

    fn capabilities(&self) -> Vec<SubAgentCapability> {
        vec![
            SubAgentCapability {
                name: "code_search".to_string(),
                description: "Search codebase for relevant symbols and patterns".to_string(),
                tools_required: vec!["read_file".to_string(), "list_files".to_string()],
                context_needed: vec!["task_description".to_string(), "project_root".to_string()],
            },
            SubAgentCapability {
                name: "symbol_lookup".to_string(),
                description: "Look up symbol definitions, usages, and relationships".to_string(),
                tools_required: vec!["read_file".to_string()],
                context_needed: vec!["related_symbols".to_string()],
            },
            SubAgentCapability {
                name: "dependency_analysis".to_string(),
                description: "Analyze file and symbol dependencies".to_string(),
                tools_required: vec!["read_file".to_string()],
                context_needed: vec!["dependencies".to_string()],
            },
            SubAgentCapability {
                name: "pattern_detection".to_string(),
                description: "Detect architectural patterns and conventions".to_string(),
                tools_required: vec!["read_file".to_string()],
                context_needed: vec!["relevant_files".to_string()],
            },
        ]
    }

    fn required_tools(&self) -> Vec<&str> {
        vec!["read_file", "list_files"]
    }

    fn can_handle(&self, task: &str) -> bool {
        let task_lower = task.to_lowercase();
        task_lower.contains("find")
            || task_lower.contains("search")
            || task_lower.contains("understand")
            || task_lower.contains("locate")
            || task_lower.contains("where")
            || task_lower.contains("what")
            || task_lower.contains("how")
            || task_lower.contains("explain")
    }

    fn required_context(&self) -> Vec<&str> {
        vec!["task_description", "project_root", "relevant_files"]
    }

    fn execute(&self, context: &SubAgentContext) -> SubAgentResult {
        let start = std::time::Instant::now();

        let mut findings = Vec::new();
        let mut files_inspected = Vec::new();
        let mut tools_used = Vec::new();
        let errors = Vec::new();
        let mut recommendations = Vec::new();

        findings.push("Research Findings".to_string());
        findings.push("Task:".to_string());
        findings.push(context.task_description.clone());
        findings.push(String::new());

        if !context.project_name.is_empty() {
            findings.push("Project:".to_string());
            findings.push(format!("  {}", context.project_name));
            if !context.project_language.is_empty() {
                findings.push(format!("  Language: {}", context.project_language));
            }
            findings.push(String::new());
        }

        findings.push("Relevant files:".to_string());
        for file in self.file_inspection(context) {
            findings.push(format!("  - {}", file));
            files_inspected.push(file);
            tools_used.push("read_file".to_string());
        }

        findings.push(String::new());
        findings.push("Relevant symbols:".to_string());
        for symbol in self.semantic_search(context) {
            findings.push(format!("  - {}", symbol));
        }

        findings.push(String::new());
        findings.push("Relevant dependencies:".to_string());
        for dep in self.dependency_analysis(context) {
            findings.push(format!("  - {}", dep));
        }

        if !context.test_files.is_empty() {
            findings.push(String::new());
            findings.push("Related tests:".to_string());
            for test in &context.test_files {
                findings.push(format!("  - {}", test));
            }
        }

        if !context.tool_observations.is_empty() {
            findings.push(String::new());
            findings.push("Tool observations:".to_string());
            for obs in &context.tool_observations {
                findings.push(format!("  - {}", truncate_for_summary(obs, 300)));
            }
        }

        if !context.memory_entries.is_empty() {
            findings.push(String::new());
            findings.push("Engineering memory:".to_string());
            for entry in &context.memory_entries {
                findings.push(format!("  - {}", truncate_for_summary(entry, 300)));
            }
        }

        findings.push(String::new());
        findings.push("Analysis:".to_string());
        if files_inspected.is_empty() {
            findings.push(
                "No repository files matched the task terms; inspect the workspace directly."
                    .to_string(),
            );
            recommendations.push("Inspect the workspace to identify relevant modules".to_string());
        } else {
            findings.push(format!(
                "Found {} relevant files, {} symbols and {} dependencies related to the task.",
                files_inspected.len(),
                context.related_symbols.len(),
                context.dependencies.len()
            ));
            findings.push("Useful existing implementations:".to_string());
            for (i, file) in files_inspected.iter().take(5).enumerate() {
                findings.push(format!("  {}. {}", i + 1, file));
            }
        }

        if !context.dependencies.is_empty() {
            recommendations.push(format!(
                "Found {} dependencies. Consider reviewing them for potential impacts.",
                context.dependencies.len()
            ));
        }

        if files_inspected.len() > 5 {
            recommendations.push("Consider narrowing the search scope".to_string());
        }

        if context.related_symbols.is_empty() {
            recommendations.push(
                "No related symbols resolved from the index. Consider building .codebro/index.db"
                    .to_string(),
            );
        }

        let duration = start.elapsed();

        SubAgentResult {
            agent_name: self.name().to_string(),
            success: errors.is_empty(),
            output: findings.join("\n"),
            files_modified: Vec::new(),
            tools_used,
            duration_ms: duration.as_millis() as u64,
            errors,
            recommendations,
        }
    }
}
