#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::agent::subagent::{SubAgent, SubAgentCapability, SubAgentContext, SubAgentResult};

pub struct PlanningAgent;

impl PlanningAgent {
    pub fn new() -> Self {
        PlanningAgent
    }

    /// Steps that reference actual repository entities from the grounded
    /// context. Empty when the context carries no repository data.
    fn grounded_steps(&self, context: &SubAgentContext) -> Vec<String> {
        let mut steps = Vec::new();

        for file in context.relevant_files.iter().take(4) {
            steps.push(format!("Inspect {}", file));
        }

        for symbol in context.related_symbols.iter().take(4) {
            steps.push(format!(
                "Analyze {} to understand current behaviour",
                symbol
            ));
        }

        for test in context.test_files.iter().take(2) {
            steps.push(format!("Add or extend regression coverage near {}", test));
        }

        if !context.dependencies.is_empty() {
            steps.push(format!(
                "Review dependency impact: {}",
                context.dependencies.join(", ")
            ));
        }

        if !context.build_info.is_empty() {
            steps.push(format!("Validate with: {}", context.build_info));
        }

        steps
    }

    fn breakdown_task(&self, context: &SubAgentContext) -> Vec<String> {
        let mut steps = Vec::new();
        let task = &context.task_description;

        if task.contains("refactor") {
            steps.push("Analyze current implementation".to_string());
            steps.push("Identify refactoring targets".to_string());
            steps.push("Plan new structure".to_string());
            steps.push("Define migration steps".to_string());
            steps.push("Plan validation approach".to_string());
        } else if task.contains("add") || task.contains("implement") {
            steps.push("Identify integration points".to_string());
            steps.push("Define interface/contract".to_string());
            steps.push("Plan implementation steps".to_string());
            steps.push("Identify test requirements".to_string());
            steps.push("Plan documentation updates".to_string());
        } else if task.contains("fix") || task.contains("bug") {
            steps.push("Reproduce the issue".to_string());
            steps.push("Identify root cause".to_string());
            steps.push("Plan fix approach".to_string());
            steps.push("Plan regression tests".to_string());
        } else if task.contains("test") {
            steps.push("Identify test targets".to_string());
            steps.push("Define test cases".to_string());
            steps.push("Plan test implementation".to_string());
        } else {
            steps.push("Analyze task requirements".to_string());
            steps.push("Identify affected components".to_string());
            steps.push("Plan execution steps".to_string());
            steps.push("Define success criteria".to_string());
        }

        steps
    }

    fn identify_risks(&self, context: &SubAgentContext) -> Vec<String> {
        let mut risks = Vec::new();

        if context.dependencies.len() > 10 {
            risks.push("High dependency count may indicate complex changes".to_string());
        }

        if context.relevant_files.len() > 10 {
            risks.push("Many files affected - changes may have wide impact".to_string());
        }

        if context.previous_results.is_empty() {
            risks
                .push("No previous research results - proceeding with limited context".to_string());
        }

        risks
    }
}

impl SubAgent for PlanningAgent {
    fn name(&self) -> &str {
        "planning"
    }

    fn purpose(&self) -> &str {
        "Create implementation plans"
    }

    fn capabilities(&self) -> Vec<SubAgentCapability> {
        vec![
            SubAgentCapability {
                name: "task_breakdown".to_string(),
                description: "Break down complex tasks into executable steps".to_string(),
                tools_required: vec![],
                context_needed: vec!["task_description".to_string()],
            },
            SubAgentCapability {
                name: "risk_assessment".to_string(),
                description: "Identify potential risks and mitigation strategies".to_string(),
                tools_required: vec![],
                context_needed: vec!["dependencies".to_string(), "relevant_files".to_string()],
            },
            SubAgentCapability {
                name: "skill_lookup".to_string(),
                description: "Find relevant skills for the task".to_string(),
                tools_required: vec![],
                context_needed: vec!["task_description".to_string()],
            },
            SubAgentCapability {
                name: "memory_recall".to_string(),
                description: "Recall similar past tasks and outcomes".to_string(),
                tools_required: vec![],
                context_needed: vec!["memory_entries".to_string()],
            },
        ]
    }

    fn required_tools(&self) -> Vec<&str> {
        vec![]
    }

    fn can_handle(&self, task: &str) -> bool {
        let task_lower = task.to_lowercase();
        task_lower.contains("plan")
            || task_lower.contains("implement")
            || task_lower.contains("add")
            || task_lower.contains("create")
            || task_lower.contains("build")
            || task_lower.contains("design")
            || task_lower.contains("refactor")
            || task_lower.contains("fix")
    }

    fn required_context(&self) -> Vec<&str> {
        vec![
            "task_description",
            "project_root",
            "relevant_files",
            "dependencies",
        ]
    }

    fn execute(&self, context: &SubAgentContext) -> SubAgentResult {
        let start = std::time::Instant::now();

        let mut output = Vec::new();
        let tools_used = Vec::new();
        let errors = Vec::new();
        let mut recommendations = Vec::new();

        output.push("Planning Phase".to_string());
        output.push("Task:".to_string());
        output.push(context.task_description.clone());
        output.push(String::new());

        let grounded = self.grounded_steps(context);
        if grounded.is_empty() {
            output.push("Execution Plan:".to_string());
            for (i, step) in self.breakdown_task(context).iter().enumerate() {
                output.push(format!("  {}. {}", i + 1, step));
            }
        } else {
            output.push("Execution Plan (grounded in repository context):".to_string());
            for (i, step) in grounded.iter().enumerate() {
                output.push(format!("  {}. {}", i + 1, step));
            }
        }

        output.push(String::new());
        output.push("Context Summary:".to_string());
        output.push(format!(
            "  Files to modify: {}",
            context.relevant_files.len()
        ));
        output.push(format!("  Dependencies: {}", context.dependencies.len()));
        output.push(format!("  Symbols: {}", context.related_symbols.len()));
        for file in context.relevant_files.iter().take(6) {
            output.push(format!("    - {}", file));
        }

        if !context.context_fragments.is_empty() {
            output.push(String::new());
            output.push("Context:".to_string());
            for fragment in context.context_fragments.iter().take(6) {
                output.push(format!("  - {}", fragment));
            }
        }

        let risks = self.identify_risks(context);
        if !risks.is_empty() {
            output.push(String::new());
            output.push("Risks Identified:".to_string());
            for risk in &risks {
                output.push(format!("  - {}", risk));
            }
        }

        if !context.previous_results.is_empty() {
            output.push(String::new());
            output.push("Previous Research Insights:".to_string());
            for result in &context.previous_results {
                if !result.recommendations.is_empty() {
                    output.push(format!("  From {}:", result.agent_name));
                    for rec in &result.recommendations {
                        output.push(format!("    - {}", rec));
                    }
                }
            }
        }

        recommendations.push("Start with research to gather more context".to_string());
        recommendations.push("Validate assumptions before implementation".to_string());
        recommendations.push("Consider adding tests for new functionality".to_string());

        let duration = start.elapsed();

        SubAgentResult {
            agent_name: self.name().to_string(),
            success: errors.is_empty(),
            output: output.join("\n"),
            files_modified: Vec::new(),
            tools_used,
            duration_ms: duration.as_millis() as u64,
            errors,
            recommendations,
        }
    }
}
