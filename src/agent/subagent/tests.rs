#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Sprint 30B: subagents must emit output grounded in real repository
//! entities (files, symbols, dependencies, tests) rather than templates.

use super::*;

/// A `SubAgentContext` populated with real repository data, as the
/// coordinator now does via `GroundingAssembler`.
fn grounded_context() -> SubAgentContext {
    SubAgentContext {
        task_description: "Trace CanonicalRuntime execution".to_string(),
        project_root: "/repo".to_string(),
        project_name: "codebro".to_string(),
        project_language: "rust".to_string(),
        workspace_summary: "Workspace root: codebro | language: rust | build tool: cargo"
            .to_string(),
        git_state: "git: branch main".to_string(),
        build_info: "build: cargo; package manager: cargo; testing: cargo test".to_string(),
        relevant_files: vec![
            "src/canonical_runtime/mod.rs".to_string(),
            "src/tui/ui.rs".to_string(),
        ],
        related_symbols: vec![
            "run_execution_loop".to_string(),
            "run_task_with_options".to_string(),
        ],
        dependencies: vec!["tokio".to_string(), "reqwest".to_string()],
        test_files: vec!["src/canonical_runtime/tests.rs".to_string()],
        tool_observations: vec!["src/canonical_runtime/mod.rs found".to_string()],
        memory_entries: vec!["language: rust".to_string()],
        context_fragments: vec!["Project codebro (rust)".to_string()],
        previous_results: Vec::new(),
    }
}

#[test]
fn test_research_output_contains_actual_file_symbol_and_dependency() {
    let agent = ResearchAgent::new();
    let result = agent.execute(&grounded_context());
    assert!(result.success);
    let out = &result.output;
    assert!(
        out.contains("src/canonical_runtime/mod.rs"),
        "research must list the real file:\n{}",
        out
    );
    assert!(
        out.contains("run_execution_loop"),
        "research must list the real symbol:\n{}",
        out
    );
    assert!(
        out.contains("tokio"),
        "research must list a real dependency:\n{}",
        out
    );
    assert!(
        out.contains("src/canonical_runtime/tests.rs"),
        "research must list related tests:\n{}",
        out
    );
}

#[test]
fn test_planning_steps_reference_actual_files() {
    let agent = PlanningAgent::new();
    let result = agent.execute(&grounded_context());
    let out = &result.output;
    assert!(
        out.contains("src/canonical_runtime/mod.rs"),
        "planning steps must reference the real file:\n{}",
        out
    );
    assert!(
        out.contains("run_execution_loop"),
        "planning steps must reference the real symbol:\n{}",
        out
    );
    assert!(
        out.contains("cargo test"),
        "planning must reference the validation command:\n{}",
        out
    );
}

#[test]
fn test_coding_strategy_references_target_files_and_symbols() {
    let agent = CodingAgent::new();
    let result = agent.execute(&grounded_context());
    let out = &result.output;
    assert!(
        out.contains("src/canonical_runtime/mod.rs"),
        "coding target must reference the real file:\n{}",
        out
    );
    assert!(
        out.contains("run_task_with_options"),
        "coding strategy must reference the real symbol:\n{}",
        out
    );
    assert!(
        out.contains("Existing implementation"),
        "coding strategy must describe the existing implementation:\n{}",
        out
    );
}

#[test]
fn test_testing_strategy_references_existing_test_files() {
    let agent = TestingAgent::new();
    let result = agent.execute(&grounded_context());
    let out = &result.output;
    assert!(
        out.contains("src/canonical_runtime/tests.rs"),
        "testing strategy must reference the real test file:\n{}",
        out
    );
    assert!(
        out.contains("cargo test"),
        "testing strategy must reference the validation command:\n{}",
        out
    );
    assert!(
        out.contains("run_execution_loop"),
        "testing strategy should propose a unit test for a real symbol:\n{}",
        out
    );
}

#[test]
fn test_review_output_references_actual_entities() {
    let agent = ReviewAgent::new();
    let result = agent.execute(&grounded_context());
    let out = &result.output;
    assert!(
        out.contains("src/canonical_runtime/mod.rs"),
        "review must reference the real file:\n{}",
        out
    );
    assert!(
        out.contains("run_execution_loop"),
        "review must reference the real symbol:\n{}",
        out
    );
    assert!(
        out.contains("tokio"),
        "review must reference a real dependency:\n{}",
        out
    );
}

#[test]
fn test_subagents_are_deterministic_with_same_context() {
    let ctx = grounded_context();
    for agent in [
        Box::new(ResearchAgent::new()) as Box<dyn SubAgent>,
        Box::new(PlanningAgent::new()) as Box<dyn SubAgent>,
        Box::new(CodingAgent::new()) as Box<dyn SubAgent>,
        Box::new(TestingAgent::new()) as Box<dyn SubAgent>,
        Box::new(ReviewAgent::new()) as Box<dyn SubAgent>,
    ] {
        let a = agent.execute(&ctx);
        let b = agent.execute(&ctx);
        assert_eq!(a.output, b.output, "{} must be deterministic", agent.name());
    }
}
