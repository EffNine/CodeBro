#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::agent::subagent::{SubAgent, SubAgentCapability, SubAgentContext, SubAgentResult};

pub struct TestingAgent;

impl TestingAgent {
    pub fn new() -> Self {
        TestingAgent
    }

    fn determine_test_strategy(&self, context: &SubAgentContext) -> String {
        let task = &context.task_description;

        if task.contains("unit") {
            "unit_testing".to_string()
        } else if task.contains("integration") {
            "integration_testing".to_string()
        } else if task.contains("e2e") || task.contains("end-to-end") {
            "e2e_testing".to_string()
        } else {
            "comprehensive_testing".to_string()
        }
    }

    fn identify_test_targets(&self, context: &SubAgentContext) -> Vec<String> {
        context.related_symbols.clone()
    }

    /// A grounded testing strategy referencing actual test files and the
    /// project's validation command.
    fn grounded_test_plan(&self, context: &SubAgentContext) -> Vec<String> {
        let mut lines = Vec::new();

        lines.push("Existing tests:".to_string());
        for test in context.test_files.iter().take(6) {
            lines.push(format!("  - {}", test));
        }
        if context.test_files.is_empty() {
            lines.push("  - (no existing test files identified)".to_string());
        }

        lines.push(String::new());
        lines.push("Recommended tests:".to_string());
        for symbol in context.related_symbols.iter().take(5) {
            lines.push(format!("  - add a unit test exercising {}", symbol));
        }
        if context.related_symbols.is_empty() {
            lines.push("  - add a new test module alongside the target source files".to_string());
        }
        if !context.test_files.is_empty() {
            lines.push(
                "  - add regression coverage to the existing test modules listed above".to_string(),
            );
        }

        lines.push(String::new());
        lines.push("Validation:".to_string());
        if context.build_info.is_empty() {
            lines.push("  - cargo test".to_string());
        } else {
            lines.push(format!("  - {}", context.build_info));
        }

        lines
    }
}

impl SubAgent for TestingAgent {
    fn name(&self) -> &str {
        "testing"
    }

    fn purpose(&self) -> &str {
        "Validate changes"
    }

    fn capabilities(&self) -> Vec<SubAgentCapability> {
        vec![
            SubAgentCapability {
                name: "run_tests".to_string(),
                description: "Execute test suites and capture results".to_string(),
                tools_required: vec!["run_command".to_string()],
                context_needed: vec!["project_root".to_string()],
            },
            SubAgentCapability {
                name: "analyze_failures".to_string(),
                description: "Analyze test failures and identify root causes".to_string(),
                tools_required: vec!["run_command".to_string(), "read_file".to_string()],
                context_needed: vec!["project_root".to_string(), "relevant_files".to_string()],
            },
            SubAgentCapability {
                name: "coverage_analysis".to_string(),
                description: "Analyze test coverage and identify gaps".to_string(),
                tools_required: vec!["run_command".to_string()],
                context_needed: vec!["project_root".to_string(), "relevant_files".to_string()],
            },
            SubAgentCapability {
                name: "regression_testing".to_string(),
                description: "Run regression tests to ensure no breaking changes".to_string(),
                tools_required: vec!["run_command".to_string()],
                context_needed: vec!["project_root".to_string()],
            },
        ]
    }

    fn required_tools(&self) -> Vec<&str> {
        vec!["run_command"]
    }

    fn can_handle(&self, task: &str) -> bool {
        let task_lower = task.to_lowercase();
        task_lower.contains("test")
            || task_lower.contains("validate")
            || task_lower.contains("verify")
            || task_lower.contains("check")
            || task_lower.contains("run")
    }

    fn required_context(&self) -> Vec<&str> {
        vec![
            "task_description",
            "project_root",
            "relevant_files",
            "files_modified",
        ]
    }

    fn execute(&self, context: &SubAgentContext) -> SubAgentResult {
        let start = std::time::Instant::now();

        let mut output = Vec::new();
        let mut tools_used = Vec::new();
        let errors = Vec::new();
        let mut recommendations = Vec::new();

        let strategy = self.determine_test_strategy(context);
        let targets = self.identify_test_targets(context);

        output.push("Testing Phase".to_string());
        output.push(format!("Strategy: {}", strategy));
        output.push(String::new());

        output.push("Task:".to_string());
        output.push(context.task_description.clone());
        output.push(String::new());

        output.push("Test Targets:".to_string());
        for target in &targets {
            output.push(format!("  - {}", target));
        }

        output.push(String::new());
        output.extend(self.grounded_test_plan(context));

        output.push(String::new());
        output.push("Test Plan:".to_string());
        match strategy.as_str() {
            "unit_testing" => {
                output.push("  1. Run unit tests for modified components".to_string());
                output.push("  2. Verify individual function behavior".to_string());
                output.push("  3. Check edge cases".to_string());
                tools_used.push("run_command".to_string());
            }
            "integration_testing" => {
                output.push("  1. Run integration tests".to_string());
                output.push("  2. Verify component interactions".to_string());
                output.push("  3. Check data flow".to_string());
                tools_used.push("run_command".to_string());
            }
            "e2e_testing" => {
                output.push("  1. Run end-to-end tests".to_string());
                output.push("  2. Verify complete workflows".to_string());
                output.push("  3. Check user-facing behavior".to_string());
                tools_used.push("run_command".to_string());
            }
            _ => {
                output.push("  1. Run comprehensive test suite".to_string());
                output.push("  2. Verify all affected components".to_string());
                output.push("  3. Check for regressions".to_string());
                tools_used.push("run_command".to_string());
            }
        }

        output.push(String::new());
        output.push("Files Under Test:".to_string());
        for file in &context.relevant_files {
            output.push(format!("  - {}", file));
        }

        recommendations.push("Run tests in isolation first".to_string());
        recommendations.push("Check test coverage".to_string());
        recommendations.push("Fix any failures before proceeding".to_string());

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
