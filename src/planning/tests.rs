//! Sprint 30E — deterministic tests for the autonomous Planning subagent.
//!
//! These tests prove:
//! - the Planning subagent performs REAL read-only verification observations
//! - Research/Testing evidence is consumed and provenance is preserved
//! - the plan is concrete: steps, files, symbols, validation, risks
//! - the reserved synthesis call produces an implementation plan
//! - all bounds are enforced (iteration / tool / model / timeout / cancel)
//! - mutating tools (create_file / edit_file / run_command / git mutation)
//!   are blocked
//! - provider failures produce a bounded error result
//! - planning never mutates repository or git state

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

use crate::agent::events::AgentEvent;
use crate::canonical_runtime::ProviderAdapter;
use crate::planning::contract::PlanningTermination;
use crate::planning::permissions::PlanningPermissionHook;
use crate::planning::permissions::PlanningTooling;
use crate::planning::{PlanningLimits, PlanningRequest, PlanningResult, PlanningSubagent};
use crate::provider_runtime::{
    Capability, CostTracker, HealthManager, IntelligentProviderRouter, ProviderId,
    ProviderRegistry, ProviderRuntime,
};
use crate::providers::Provider;
use crate::research::ResearchResult;
use crate::testing::{TestCommandResult, TestingResult};
use crate::tools::hooks::PermissionHook;

// =========================================================================
// Mock provider
// =========================================================================

/// A scripted mock provider for planning tests. Consumes responses
/// sequentially and records every prompt it receives (so tests can prove the
/// evidence trail flows into the next model call).
#[derive(Clone)]
struct PlanningMockProvider {
    name: String,
    model: String,
    responses: Arc<Mutex<Vec<String>>>,
    prompts: Arc<Mutex<Vec<String>>>,
    structured: bool,
    fail: Arc<AtomicBool>,
}

impl PlanningMockProvider {
    fn text(name: &str, responses: Vec<String>) -> Self {
        PlanningMockProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            responses: Arc::new(Mutex::new(responses)),
            prompts: Arc::new(Mutex::new(Vec::new())),
            structured: false,
            fail: Arc::new(AtomicBool::new(false)),
        }
    }

    fn failing(name: &str) -> Self {
        let p = PlanningMockProvider::text(name, Vec::new());
        p.fail.store(true, Ordering::SeqCst);
        p
    }

    fn next(&self) -> String {
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            String::new()
        } else {
            responses.remove(0)
        }
    }

    fn prompt_log(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

impl Provider for PlanningMockProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn base_url(&self) -> &str {
        "mock://planning"
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn api_key(&self) -> Option<&str> {
        None
    }
    fn supports_function_calling(&self) -> bool {
        self.structured
    }
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Streaming, Capability::ToolCalling]
    }
    fn send_message(
        &self,
        _m: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
    {
        let response = self.next();
        Box::pin(async move { Ok(response) })
    }
    fn stream_response(
        &self,
        message: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                > + Send
                + '_,
        >,
    > {
        self.prompts.lock().unwrap().push(message.to_string());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        if self.fail.load(Ordering::SeqCst) {
            let result = Err(anyhow::anyhow!("planning mock provider offline"));
            Box::pin(async move { result })
        } else {
            let response = self.next();
            let _ = tx.send(response);
            Box::pin(async move { Ok(rx) })
        }
    }
    fn stream_response_with_tools(
        &self,
        message: &str,
        _tools: &[crate::providers::ToolDefinition],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<(String, Vec<crate::providers::StructuredToolCall>)>,
                > + Send
                + '_,
        >,
    > {
        self.prompts.lock().unwrap().push(message.to_string());
        let response = self.next();
        Box::pin(async move {
            let calls: Vec<crate::providers::StructuredToolCall> =
                if response.trim().is_empty() || response == "[]" {
                    Vec::new()
                } else {
                    let arr: Vec<serde_json::Value> = serde_json::from_str(&response)?;
                    arr.into_iter()
                        .map(|item| {
                            let id = item
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = item["function"]["name"].as_str().unwrap_or("").to_string();
                            let arguments = item["function"]["arguments"]
                                .as_str()
                                .unwrap_or("")
                                .to_string();
                            crate::providers::StructuredToolCall {
                                id,
                                name,
                                arguments,
                            }
                        })
                        .collect()
                };
            Ok((String::new(), calls))
        })
    }
}

// =========================================================================
// Harness
// =========================================================================

struct PlanningHarness {
    provider_runtime: ProviderRuntime,
    router: IntelligentProviderRouter,
    io_providers: HashMap<ProviderId, Arc<dyn Provider>>,
}

impl PlanningHarness {
    fn new(provider: Arc<dyn Provider>) -> Self {
        let registry = ProviderRegistry::new();
        let health = HealthManager::new();
        let cost = CostTracker::new();
        let provider_runtime =
            ProviderRuntime::from_parts(registry.clone(), health.clone(), cost.clone());
        let router = IntelligentProviderRouter::new(registry.clone(), health.clone(), cost.clone());
        let adapter = ProviderAdapter::new(provider.clone());
        registry.register(&adapter).unwrap();
        provider_runtime
            .circuit_breakers()
            .get_or_create(adapter.provider_id());
        let mut io_providers = HashMap::new();
        io_providers.insert(adapter.provider_id().clone(), provider);
        PlanningHarness {
            provider_runtime,
            router,
            io_providers,
        }
    }

    fn subagent(self, root: &Path) -> PlanningSubagent {
        let tooling = PlanningTooling::new(root);
        PlanningSubagent::new(
            self.provider_runtime,
            self.router,
            self.io_providers,
            tooling,
        )
    }
}

fn event_sink() -> (
    Arc<Mutex<Vec<AgentEvent>>>,
    Box<dyn Fn(AgentEvent) + Send + Sync>,
) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    (events, Box::new(move |e| sink.lock().unwrap().push(e)))
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

/// A fixture workspace that mirrors the canonical-runtime research fixture:
/// the runtime execution path lives in src/canonical_runtime/mod.rs.
fn planning_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n[dependencies]\ntokio = \"1\"\n",
    );
    write(
        &dir.path().join("src/canonical_runtime/mod.rs"),
        "pub struct CanonicalRuntime {}\nimpl CanonicalRuntime {\n    pub fn run_execution_loop() {}\n    pub fn stream_once() {}\n}\n",
    );
    write(
        &dir.path().join("src/canonical_runtime/tests.rs"),
        "#[cfg(test)]\nmod tests { #[test] fn execution_works() {} }\n",
    );
    dir
}

/// A plausible ResearchResult: research claims the execution path lives in
/// src/canonical_runtime/mod.rs and surfaced `run_execution_loop`.
fn research_result() -> ResearchResult {
    ResearchResult {
        summary: "The canonical runtime execution path lives in src/canonical_runtime/mod.rs."
            .to_string(),
        findings: Vec::new(),
        files_inspected: vec![PathBuf::from("src/canonical_runtime/mod.rs")],
        symbols_found: vec!["run_execution_loop".to_string(), "stream_once".to_string()],
        tool_calls: 1,
        iterations: 1,
        model_calls: 2,
        termination: crate::research::ResearchTermination::Completed,
        synthesis_complete: true,
        tool_observations: Vec::new(),
        limitations: Vec::new(),
        duration_ms: 100,
        output_size: 10,
        provider: "mock".to_string(),
        model: "mock-model".to_string(),
    }
}

/// A plausible TestingResult: `cargo test` passed with exit code 0.
fn testing_result() -> TestingResult {
    TestingResult {
        summary: "all tests pass".to_string(),
        findings: Vec::new(),
        commands_run: vec![TestCommandResult {
            command: "cargo test".to_string(),
            working_directory: "/repo".to_string(),
            exit_code: 0,
            success: true,
            duration_ms: 1500,
            output: "1 passed; 0 failed".to_string(),
            timeout: false,
            cancelled: false,
            denied: false,
            denied_reason: None,
        }],
        files_inspected: Vec::new(),
        failures: Vec::new(),
        tool_calls: 1,
        iterations: 1,
        model_calls: 2,
        termination: crate::testing::TestingTermination::Completed,
        synthesis_complete: true,
        observations: Vec::new(),
        limitations: Vec::new(),
        duration_ms: 2000,
        output_size: 100,
        provider: "mock".to_string(),
        model: "mock-model".to_string(),
        git_before: None,
        git_after: None,
    }
}

/// A rich, concrete final implementation plan (the scripted model output).
fn concrete_plan_answer() -> String {
    "## Existing implementation
The CanonicalRuntime tool execution path is implemented in src/canonical_runtime/mod.rs.

## Required change
run_execution_loop should propagate tool errors before falling back to the output tail.

Step 1: Modify run_execution_loop in src/canonical_runtime/mod.rs
Files: src/canonical_runtime/mod.rs
Symbols: run_execution_loop, stream_once
Reason: the execution path currently catches tool errors and returns the tail output
Depends: src/canonical_runtime/mod.rs
Validate: cargo test canonical_runtime; cargo check
Tests: src/canonical_runtime/tests.rs
Risk: changing run_execution_loop may affect existing callers

Dependencies: canonical_runtime
Assumption: no external caller depends on the tail-output fallback
Risk: callers of run_execution_loop may break (severity: medium) [mitigation: add regression coverage]
"
    .to_string()
}

async fn run_planning(
    harness: PlanningHarness,
    task: &str,
    root: &Path,
    limits: Option<PlanningLimits>,
    research: Option<ResearchResult>,
    testing: Option<TestingResult>,
) -> PlanningResult {
    let (_, emit) = event_sink();
    let mut subagent = harness.subagent(root);
    let mut request = PlanningRequest::new(task, root)
        .with_research(research)
        .with_testing(testing);
    if let Some(limits) = limits {
        request = request.with_limits(limits);
    }
    subagent.run(request, &emit, None).await
}

// =========================================================================
// Permission: read-only boundary
// =========================================================================

#[test]
fn test_planning_allows_read_only_tools() {
    let hook = PlanningPermissionHook::new();
    for tool in ["list_files", "read_file", "git_status", "git_diff"] {
        assert!(hook.allows(tool), "{} must be allowed", tool);
    }
}

#[test]
fn test_planning_denies_create_file() {
    let hook = PlanningPermissionHook::new();
    assert!(!hook.allows("create_file"));
    let ctx = crate::tools::context::ToolContext::new("create_file", "{}");
    assert!(hook.check(&ctx).is_denied());
}

#[test]
fn test_planning_denies_edit_file() {
    let hook = PlanningPermissionHook::new();
    assert!(!hook.allows("edit_file"));
    let ctx = crate::tools::context::ToolContext::new("edit_file", "{}");
    assert!(hook.check(&ctx).is_denied());
}

#[test]
fn test_planning_denies_run_command() {
    let hook = PlanningPermissionHook::new();
    assert!(
        !hook.allows("run_command"),
        "planning must never execute commands"
    );
    let ctx = crate::tools::context::ToolContext::new("run_command", "cargo test");
    assert!(hook.check(&ctx).is_denied());
}

#[test]
fn test_planning_denies_git_mutation() {
    let hook = PlanningPermissionHook::new();
    for tool in [
        "git_commit",
        "git_commit_all",
        "git_checkout",
        "git_reset",
        "git_push",
        "git_rebase",
    ] {
        assert!(!hook.allows(tool), "{} must not be allowed", tool);
        let ctx = crate::tools::context::ToolContext::new(tool, "{}");
        assert!(hook.check(&ctx).is_denied(), "{} must be denied", tool);
    }
}

#[test]
fn test_planning_registry_only_contains_allowed_tools() {
    let dir = tempfile::tempdir().unwrap();
    let tooling = PlanningTooling::new(dir.path());
    let names = tooling.registry.names();
    assert_eq!(
        names.len(),
        4,
        "registry must expose exactly 4 tools: {:?}",
        names
    );
    for allowed in ["list_files", "read_file", "git_status", "git_diff"] {
        assert!(names.contains(&allowed.to_string()));
    }
    for denied in [
        "create_file",
        "edit_file",
        "run_command",
        "git_commit",
        "playwright",
    ] {
        assert!(!names.contains(&denied.to_string()));
    }
}

#[tokio::test]
async fn test_planning_denied_mutations_recorded_as_failed_observations() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![
            r#"<invoke name="create_file">{"path": "src/evil.rs", "content": "boom"}</invoke>"#
                .to_string(),
            r#"<invoke name="edit_file">{"path": "src/canonical_runtime/mod.rs", "content": "boom"}</invoke>"#
                .to_string(),
            r#"<invoke name="run_command">{"command": "git commit -m x"}</invoke>"#.to_string(),
            "The plan is complete. No files were modified.".to_string(),
        ],
    )));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        None,
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::Completed);
    for attempted in ["create_file", "edit_file", "run_command"] {
        let attempt = result
            .tool_observations
            .iter()
            .find(|o| o.name == attempted)
            .expect("attempt must be recorded");
        assert!(
            attempt.result.starts_with("Error"),
            "{} must be rejected, got: {}",
            attempted,
            attempt.result
        );
        assert!(!attempt.success);
    }
    assert!(
        !dir.path().join("src/evil.rs").exists(),
        "create_file must not create anything"
    );
}

// =========================================================================
// Autonomous execution: real read-only observations
// =========================================================================

#[tokio::test]
async fn test_planning_performs_real_read_observation() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            "Step 1: Modify run_execution_loop in src/canonical_runtime/mod.rs\nFiles: src/canonical_runtime/mod.rs\nSymbols: run_execution_loop\nReason: error handling\nValidate: cargo test\nRisk: x".to_string(),
        ],
    )));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        None,
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::Completed);
    assert!(
        result.tool_calls >= 1,
        "planning must execute at least one tool"
    );
    let read = result
        .tool_observations
        .iter()
        .find(|o| o.name == "read_file")
        .expect("read_file observation present");
    assert!(
        read.result.contains("run_execution_loop"),
        "read_file must return ACTUAL file contents, got: {}",
        read.result
    );
    assert!(read.success);
    assert!(
        result
            .tool_observations
            .iter()
            .any(|o| o.arguments.contains("canonical_runtime")),
        "inspected files must be recorded in the observations, got: {:?}",
        result.tool_observations
    );
}

/// Planning should VERIFY, not rediscover: given research evidence for
/// src/canonical_runtime/mod.rs, the session performs exactly ONE targeted
/// read of that file and nothing else (no broad list_files scan).
#[tokio::test]
async fn test_planning_reads_targeted_file() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            concrete_plan_answer(),
        ],
    )));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        None,
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::Completed);
    assert_eq!(
        result.tool_calls, 1,
        "planning must use exactly one targeted read"
    );
    assert!(
        result
            .tool_observations
            .iter()
            .all(|o| o.name == "read_file"),
        "no broad scans: observations = {:?}",
        result
            .tool_observations
            .iter()
            .map(|o| o.name.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        result
            .tool_observations
            .iter()
            .any(|o| o.arguments.contains("canonical_runtime")),
        "the targeted read must inspect the research-named file"
    );
}

#[tokio::test]
async fn test_planning_tool_result_reaches_next_iteration() {
    let dir = planning_workspace();
    let provider = Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/tests.rs"}</invoke>"#
                .to_string(),
            concrete_plan_answer(),
        ],
    ));
    let harness = PlanningHarness::new(provider.clone());
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        None,
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::Completed);
    assert_eq!(result.iterations, 3);
    assert_eq!(result.tool_calls, 2);

    // Iteration 2's prompt contains iteration 1's read result; iteration 3's
    // prompt contains iteration 2's read result.
    let prompts = provider.prompt_log();
    assert_eq!(prompts.len(), 3);
    assert!(
        prompts[1].contains("run_execution_loop"),
        "iteration 2 prompt must contain the first read result:\n{}",
        prompts[1]
    );
    assert!(
        prompts[1].contains("CURRENT PLANNING OBSERVATIONS"),
        "iteration 2 prompt must carry the observation section"
    );
    assert!(
        prompts[2].contains("execution_works"),
        "iteration 3 prompt must contain the tests.rs read result"
    );
}

// =========================================================================
// Evidence integration: Research and Testing enter Planning
// =========================================================================

#[tokio::test]
async fn test_planning_consumes_research_result() {
    let dir = planning_workspace();
    let provider = Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![concrete_plan_answer()],
    ));
    let harness = PlanningHarness::new(provider.clone());
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        None,
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::Completed);
    let prompt = provider.prompt_log().remove(0);
    assert!(
        prompt.contains("RESEARCH EVIDENCE"),
        "the prompt must carry a distinct RESEARCH EVIDENCE section:\n{}",
        prompt
    );
    assert!(
        prompt.contains("src/canonical_runtime/mod.rs"),
        "research files must reach the planning prompt"
    );
    assert!(
        prompt.contains("run_execution_loop"),
        "research symbols must reach the planning prompt"
    );
}

#[tokio::test]
async fn test_planning_consumes_testing_result() {
    let dir = planning_workspace();
    let provider = Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![concrete_plan_answer()],
    ));
    let harness = PlanningHarness::new(provider.clone());
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        None,
        Some(testing_result()),
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::Completed);
    let prompt = provider.prompt_log().remove(0);
    assert!(
        prompt.contains("TESTING EVIDENCE"),
        "the prompt must carry a distinct TESTING EVIDENCE section:\n{}",
        prompt
    );
    assert!(
        prompt.contains("cargo test"),
        "testing commands must reach the planning prompt"
    );
    assert!(
        prompt.contains("exit_code: 0"),
        "authoritative exit codes must reach the planning prompt"
    );
    assert!(
        prompt.contains("AUTHORITATIVE"),
        "the prompt must mark the machine facts as authoritative"
    );
}

/// The multi-evidence test (Sprint 30E §16): Planning receives BOTH a
/// ResearchResult and a TestingResult; the resulting plan references both
/// evidence sources and provenance is preserved in `result.evidence`.
#[tokio::test]
async fn test_planning_preserves_evidence_provenance() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![concrete_plan_answer()],
    )));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        Some(testing_result()),
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::Completed);
    // The evidence trail carries both sources.
    let research_entries: Vec<_> = result
        .evidence
        .iter()
        .filter(|e| e.source == "research")
        .collect();
    let testing_entries: Vec<_> = result
        .evidence
        .iter()
        .filter(|e| e.source == "testing")
        .collect();
    assert!(
        research_entries
            .iter()
            .any(|e| e.reference.contains("canonical_runtime")),
        "research evidence must reference the inspected file: {:?}",
        research_entries
            .iter()
            .map(|e| &e.reference)
            .collect::<Vec<_>>()
    );
    assert!(
        testing_entries.iter().any(|e| e.reference == "cargo test"),
        "testing evidence must reference the cargo test command"
    );
    let testing_entry = testing_entries
        .iter()
        .find(|e| e.reference == "cargo test")
        .unwrap();
    assert!(
        testing_entry.summary.contains("exit_code 0"),
        "testing evidence must preserve the authoritative exit code: {}",
        testing_entry.summary
    );
    // The plan step references both sources.
    assert_eq!(result.plan.len(), 1);
    assert!(
        result.plan[0]
            .evidence
            .iter()
            .any(|e| e.contains("[research]")),
        "step evidence must cite research: {:?}",
        result.plan[0].evidence
    );
    assert!(
        result.plan[0]
            .evidence
            .iter()
            .any(|e| e.contains("[testing]")),
        "step evidence must cite the authoritative testing facts: {:?}",
        result.plan[0].evidence
    );
}

/// The verify-not-rediscover chain: research claims `run_execution_loop` lives
/// in src/canonical_runtime/mod.rs; the planner VERIFIES the claim with a
/// targeted read and the final step cites both observations.
#[tokio::test]
async fn test_planning_verifies_research_claim_with_read() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            concrete_plan_answer(),
        ],
    )));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        Some(testing_result()),
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::Completed);
    // The planner actually read the claimed file.
    let read = result
        .tool_observations
        .iter()
        .find(|o| o.name == "read_file")
        .expect("the research claim must be verified with a real read");
    assert!(
        read.result.contains("run_execution_loop"),
        "the read must confirm the research claim, got: {}",
        read.result
    );
    // The step cites the research claim AND the planner's own verification.
    assert!(
        result.plan[0]
            .evidence
            .iter()
            .any(|e| e.contains("[research]") && e.contains("inspected by research")),
        "step must preserve the research provenance: {:?}",
        result.plan[0].evidence
    );
    assert!(
        result.plan[0]
            .evidence
            .iter()
            .any(|e| e.contains("[planning_read]")),
        "step must cite the planner's own verification read: {:?}",
        result.plan[0].evidence
    );
}

// =========================================================================
// Plan structure: concrete steps, files, symbols, validation, risks
// =========================================================================

#[tokio::test]
async fn test_planning_result_contains_concrete_plan_steps() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![concrete_plan_answer()],
    )));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        Some(testing_result()),
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::Completed);
    assert!(result.synthesis_complete);
    assert_eq!(result.plan.len(), 1, "one concrete step expected");
    assert_eq!(result.plan[0].order, 1);
    assert_ne!(result.plan[0].action, "", "steps must have an action");
    assert!(
        result.plan[0].action.to_lowercase().contains("modify"),
        "the action must be concrete, got: {}",
        result.plan[0].action
    );
    assert!(
        !result.plan[0]
            .action
            .to_lowercase()
            .contains("analyze the code"),
        "vague analysis steps are not plans"
    );
}

#[tokio::test]
async fn test_planning_steps_reference_real_files() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![concrete_plan_answer()],
    )));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        None,
    )
    .await;

    assert!(
        result.plan[0]
            .target_files
            .iter()
            .any(|f| f.to_string_lossy() == "src/canonical_runtime/mod.rs"),
        "the step must name the real file, got: {:?}",
        result.plan[0].target_files
    );
    assert!(
        result
            .affected_files
            .iter()
            .any(|f| f.to_string_lossy().contains("canonical_runtime")),
        "affected_files must be derived from the steps, got: {:?}",
        result.affected_files
    );
}

#[tokio::test]
async fn test_planning_steps_reference_real_symbols() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![concrete_plan_answer()],
    )));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        None,
    )
    .await;

    assert!(
        result.plan[0]
            .target_symbols
            .contains(&"run_execution_loop".to_string()),
        "the step must name the real symbol, got: {:?}",
        result.plan[0].target_symbols
    );
    assert!(
        result
            .affected_symbols
            .contains(&"run_execution_loop".to_string()),
        "affected_symbols must be derived from the steps"
    );
}

#[tokio::test]
async fn test_planning_includes_validation() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![concrete_plan_answer()],
    )));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        Some(testing_result()),
    )
    .await;

    assert!(
        result.plan[0]
            .validation
            .iter()
            .any(|v| v.contains("cargo test")),
        "the step must carry concrete validation commands, got: {:?}",
        result.plan[0].validation
    );
}

#[tokio::test]
async fn test_planning_includes_risks() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![concrete_plan_answer()],
    )));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        None,
    )
    .await;

    assert!(
        !result.risks.is_empty(),
        "the plan must surface risks, got: {:?}",
        result.risks
    );
    assert!(
        result.risks.iter().any(|r| r.description.contains("break")),
        "the risk must be concrete, got: {:?}",
        result
            .risks
            .iter()
            .map(|r| &r.description)
            .collect::<Vec<_>>()
    );
    assert!(
        result.risks.iter().any(|r| r.severity == "medium"),
        "severity must be extracted"
    );
}

#[tokio::test]
async fn test_planning_includes_tests_to_update() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![concrete_plan_answer()],
    )));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        None,
    )
    .await;

    assert!(
        result
            .tests_to_update
            .iter()
            .any(|t| t.to_string_lossy().contains("tests.rs")),
        "tests_to_update must carry the test targets, got: {:?}",
        result.tests_to_update
    );
    assert!(
        !result.assumptions.is_empty() && result.assumptions[0].contains("external caller"),
        "assumptions must be surfaced separately from facts, got: {:?}",
        result.assumptions
    );
}

// =========================================================================
// Synthesis: reserved final plan call (Sprint 30E)
// =========================================================================

/// A model that keeps reading must be forced into the final implementation
/// plan once the evidence budget is exhausted, and that call must produce the
/// structured plan.
#[tokio::test]
async fn test_planning_reserves_final_synthesis_call() {
    let dir = planning_workspace();
    let provider = Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            concrete_plan_answer(),
        ],
    ));
    let harness = PlanningHarness::new(provider.clone());
    let limits = PlanningLimits {
        max_model_calls: 3,
        reserved_synthesis_calls: 1,
        ..PlanningLimits::default()
    };
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        Some(limits),
        Some(research_result()),
        None,
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::Completed);
    assert!(
        result.synthesis_complete,
        "the reserved synthesis call must produce the final plan"
    );
    assert_eq!(result.model_calls, 3, "3 calls: 2 evidence + 1 synthesis");
    assert_eq!(
        result.tool_calls, 2,
        "evidence gathering must stop at the budget"
    );
    assert_eq!(result.plan.len(), 1, "the synthesis must produce the plan");

    let prompts = provider.prompt_log();
    assert_eq!(prompts.len(), 3);
    assert!(
        prompts[2].contains("FINAL IMPLEMENTATION PLAN"),
        "the last call must be the synthesis prompt:\n{}",
        prompts[2]
    );
    assert!(
        !prompts[2].contains("PLANNING STEP 3"),
        "synthesis prompt must not use the evidence step format"
    );
}

/// If the model uses the reserved synthesis call to request MORE evidence,
/// the loop must terminate honestly: `ModelLimit`, `synthesis_complete =
/// false`, with the evidence trail preserved and no fabricated plan.
#[tokio::test]
async fn test_planning_synthesis_cannot_request_tools() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
        ],
    )));
    let limits = PlanningLimits {
        max_model_calls: 3,
        reserved_synthesis_calls: 1,
        ..PlanningLimits::default()
    };
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        Some(limits),
        Some(research_result()),
        None,
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::ModelLimit);
    assert!(!result.synthesis_complete);
    assert!(result.plan.is_empty(), "no plan may be fabricated");
    assert_eq!(result.tool_calls, 2);
    assert!(
        result.evidence.iter().any(|e| e.source == "planning_read"),
        "the evidence trail must be preserved"
    );
    assert!(
        result.summary.contains("model_limit"),
        "the summary must state the honest termination, got: {}",
        result.summary
    );
    assert!(
        result.limitations.iter().any(|l| l.contains("model-call")),
        "limitations must record the model-call limit"
    );
}

/// A budget too small to gather evidence must not fabricate a plan: it
/// terminates at `ModelLimit` with `synthesis_complete = false`.
#[tokio::test]
async fn test_planning_no_evidence_synthesis_is_honest() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string()],
    )));
    let limits = PlanningLimits {
        max_model_calls: 1,
        reserved_synthesis_calls: 1,
        ..PlanningLimits::default()
    };
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        Some(limits),
        None,
        None,
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::ModelLimit);
    assert!(!result.synthesis_complete);
    assert!(result.plan.is_empty(), "no evidence → no fabricated plan");
    assert!(result.tool_calls <= 1);
}

/// A planner that voluntarily stops exploring (no tool call with full input
/// evidence) completes with the final plan — no forced call is needed.
#[tokio::test]
async fn test_planning_voluntary_completion_is_full_synthesis() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![concrete_plan_answer()],
    )));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        Some(testing_result()),
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::Completed);
    assert!(result.synthesis_complete);
    assert_eq!(result.model_calls, 1);
    assert_eq!(result.plan.len(), 1);
}

// =========================================================================
// Bounds
// =========================================================================

#[tokio::test]
async fn test_planning_iteration_limit() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
        ],
    )));
    let limits = PlanningLimits {
        max_iterations: 1,
        ..PlanningLimits::default()
    };
    let result = run_planning(
        harness,
        "plan forever",
        dir.path(),
        Some(limits),
        Some(research_result()),
        None,
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::IterationLimit);
    assert!(result.iterations <= 1);
}

#[tokio::test]
async fn test_planning_tool_call_limit() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>
<invoke name="read_file">{"path": "src/canonical_runtime/tests.rs"}</invoke>"#
                .to_string(),
        ],
    )));
    let limits = PlanningLimits {
        max_tool_calls: 1,
        ..PlanningLimits::default()
    };
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        Some(limits),
        Some(research_result()),
        None,
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::ToolLimit);
    assert!(result.tool_calls <= 1);
}

#[tokio::test]
async fn test_planning_model_call_limit() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
        ],
    )));
    let limits = PlanningLimits {
        max_model_calls: 1,
        ..PlanningLimits::default()
    };
    let result = run_planning(
        harness,
        "plan forever",
        dir.path(),
        Some(limits),
        Some(research_result()),
        None,
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::ModelLimit);
    assert!(result.model_calls <= 1);
}

#[tokio::test]
async fn test_planning_timeout() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec!["I never finish this.".to_string()],
    )));
    let limits = PlanningLimits {
        timeout_ms: 0,
        ..PlanningLimits::default()
    };
    let result = run_planning(
        harness,
        "plan slowly",
        dir.path(),
        Some(limits),
        Some(research_result()),
        None,
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::Timeout);
    assert_eq!(result.iterations, 0);
}

#[tokio::test]
async fn test_planning_cancellation() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec!["I am slow.".to_string()],
    )));
    let (_, emit) = event_sink();
    let token = crate::cancellation::CancellationToken::new();
    token.cancel();
    let mut subagent = harness.subagent(dir.path());
    let request = PlanningRequest::new("plan cancelled", dir.path());
    let result = subagent.run(request, &emit, Some(token)).await;

    assert_eq!(result.termination, PlanningTermination::Cancelled);
    assert!(result.limitations.iter().any(|l| l.contains("cancel")));
}

// =========================================================================
// Failure isolation (subagent level)
// =========================================================================

#[tokio::test]
async fn test_planning_provider_failure_is_bounded() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::failing("planning-fail")));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        None,
    )
    .await;

    // The error result is structured and bounded — never a panic.
    assert_eq!(result.termination, PlanningTermination::Error);
    assert!(result
        .limitations
        .iter()
        .any(|l| l.contains("planning mock provider offline")));
    assert!(result.render().contains("Autonomous Planning"));
    assert_eq!(result.tool_calls, 0);
    assert!(!result.synthesis_complete);
}

// =========================================================================
// No mutation: real git fixture
// =========================================================================

fn git_fixture() -> tempfile::TempDir {
    let dir = planning_workspace();
    let out = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git init must succeed");
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "test@test"])
        .current_dir(dir.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["commit", "-q", "-m", "initial"])
        .current_dir(dir.path())
        .output();
    dir
}

fn git_status(root: &Path) -> String {
    let output = std::process::Command::new("git")
        .args(["status", "--short"])
        .current_dir(root)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Before/after tracked files and git state must match. The model attempts
/// every mutating surface; all must be denied and nothing may change.
#[tokio::test]
async fn test_planning_never_mutates_repository_state() {
    let dir = git_fixture();
    let status_before = git_status(dir.path());
    let file_before =
        std::fs::read_to_string(dir.path().join("src/canonical_runtime/mod.rs")).unwrap();

    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![
            r#"<invoke name="create_file">{"path": "src/evil.rs", "content": "fn evil() {}"}</invoke>"#
                .to_string(),
            r#"<invoke name="edit_file">{"path": "src/canonical_runtime/mod.rs", "content": "boom"}</invoke>"#
                .to_string(),
            r#"<invoke name="run_command">{"command": "git commit -am x"}</invoke>"#.to_string(),
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            concrete_plan_answer(),
        ],
    )));
    let result = run_planning(
        harness,
        "plan the refactor",
        dir.path(),
        None,
        Some(research_result()),
        None,
    )
    .await;

    assert_eq!(result.termination, PlanningTermination::Completed);
    // Every mutation attempt was denied (recorded as failed observations).
    for attempted in ["create_file", "edit_file", "run_command"] {
        let attempt = result
            .tool_observations
            .iter()
            .find(|o| o.name == attempted)
            .expect("mutation attempt must be recorded");
        assert!(!attempt.success, "{} must be denied", attempted);
    }
    // Git state and tracked file contents are unchanged.
    let status_after = git_status(dir.path());
    assert_eq!(
        status_before, status_after,
        "planning must not mutate git state\nbefore: {status_before}\nafter: {status_after}"
    );
    let file_after =
        std::fs::read_to_string(dir.path().join("src/canonical_runtime/mod.rs")).unwrap();
    assert_eq!(file_before, file_after, "tracked file contents unchanged");
    assert!(
        !dir.path().join("src/evil.rs").exists(),
        "no file may be created"
    );
}

// =========================================================================
// Observability
// =========================================================================

#[tokio::test]
async fn test_planning_emits_distinguishable_events() {
    let dir = planning_workspace();
    let harness = PlanningHarness::new(Arc::new(PlanningMockProvider::text(
        "planning-mock",
        vec![
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            concrete_plan_answer(),
        ],
    )));
    let (events, emit) = {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let sink_clone = sink.clone();
        (
            sink,
            Box::new(move |e| sink_clone.lock().unwrap().push(e))
                as Box<dyn Fn(AgentEvent) + Send + Sync>,
        )
    };
    let mut subagent = harness.subagent(dir.path());
    let request = PlanningRequest::new("plan the refactor", dir.path());
    let _ = subagent.run(request, &emit, None).await;

    let evs = events.lock().unwrap();
    assert!(
        evs.iter()
            .any(|e| matches!(e, AgentEvent::AgentStarted { agent, .. } if agent == "planning")),
        "planning started event must be emitted"
    );
    assert!(
        evs.iter().any(|e| matches!(e, AgentEvent::AgentStatusChanged { agent, status: crate::agent::status::AgentStatus::Planning } if agent == "planning")),
        "planning status event must be emitted"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, AgentEvent::ToolStarted { tool, .. } if tool == "read_file")),
        "tool started events must be emitted"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, AgentEvent::ToolCompleted { tool, .. } if tool == "read_file")),
        "tool completed events must be emitted"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, AgentEvent::AgentCompleted { agent, .. } if agent == "planning")),
        "planning completed event must be emitted"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, AgentEvent::Log { level, .. } if level == "planning")),
        "planning log events must be emitted"
    );
}
