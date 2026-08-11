//! Sprint 30D — deterministic tests for the autonomous Testing subagent.
//!
//! These tests prove:
//! - Testing executes REAL policy-checked validation commands and captures the
//!   authoritative PTY exit code
//! - the exit code is authoritative over model/output prose
//! - command results affect the next iteration (the evidence trail)
//! - the restricted registry blocks mutating tools and destructive commands
//! - all bounds are enforced (iteration / tool / model / timeout / cancel /
//!   per-command timeout)
//! - provider failures produce a bounded error result
//! - testing never mutates repository or git state (ignored build artifacts
//!   such as `target/` are allowed)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

use crate::agent::events::AgentEvent;
use crate::canonical_runtime::ProviderAdapter;
use crate::provider_runtime::{
    Capability, CostTracker, HealthManager, IntelligentProviderRouter, ProviderId,
    ProviderRegistry, ProviderRuntime, RouteRequest,
};
use crate::providers::Provider;

use super::limits::TestingLimits;
use super::permissions::TestingTooling;
use super::TestingRequest;
use super::TestingResult;
use super::TestingSubagent;
use super::TestingTermination;

// =========================================================================
// Mock provider
// =========================================================================

/// A scripted mock provider for testing tests. Consumes responses sequentially
/// and records every prompt it receives (so tests can prove the evidence trail
/// flows into the next model call).
#[derive(Clone)]
struct TestingMockProvider {
    name: String,
    model: String,
    responses: Arc<Mutex<Vec<String>>>,
    prompts: Arc<Mutex<Vec<String>>>,
    structured: bool,
    fail: Arc<AtomicBool>,
}

impl TestingMockProvider {
    fn text(name: &str, responses: Vec<String>) -> Self {
        TestingMockProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            responses: Arc::new(Mutex::new(responses)),
            prompts: Arc::new(Mutex::new(Vec::new())),
            structured: false,
            fail: Arc::new(AtomicBool::new(false)),
        }
    }

    fn structured(name: &str, responses: Vec<String>) -> Self {
        TestingMockProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            responses: Arc::new(Mutex::new(responses)),
            prompts: Arc::new(Mutex::new(Vec::new())),
            structured: true,
            fail: Arc::new(AtomicBool::new(false)),
        }
    }

    fn failing(name: &str) -> Self {
        let p = TestingMockProvider::text(name, Vec::new());
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

impl Provider for TestingMockProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn base_url(&self) -> &str {
        "mock://testing"
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
            let result = Err(anyhow::anyhow!("testing mock provider offline"));
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

struct TestingHarness {
    provider_runtime: ProviderRuntime,
    router: IntelligentProviderRouter,
    io_providers: HashMap<ProviderId, Arc<dyn Provider>>,
}

impl TestingHarness {
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
        TestingHarness {
            provider_runtime,
            router,
            io_providers,
        }
    }

    fn subagent(self, root: &Path, command_timeout_secs: u64) -> TestingSubagent {
        let tooling = TestingTooling::new(root, command_timeout_secs);
        TestingSubagent::new(
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

/// A fixture workspace with a real (non-compiling) project layout for
/// permission / denial / limit tests.
fn fixture_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&dir.path().join(".gitignore"), "target/\nCargo.lock\n");
    write(
        &dir.path().join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    );
    write(
        &dir.path().join("src/parser.rs"),
        "pub fn parse_tool_calls() {}\n",
    );
    dir
}

/// A fixture crate that COMPILES and whose tests all pass. One test is named
/// with the substring "failed" so its output contains "failed" while the exit
/// code is 0 — proving exit code authority.
fn fixture_passing_crate() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join("Cargo.toml"),
        "[package]\nname = \"vt\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&dir.path().join(".gitignore"), "target/\nCargo.lock\n");
    write(
        &dir.path().join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn ok() {}\n    #[test]\n    fn failed_lol() {}\n}\n",
    );
    dir
}

/// A fixture crate that COMPILES but whose single test always fails with a
/// panic message containing the substring "passed" — a failing `cargo test`
/// whose output contains "passed" must still be a failure (exit code 101).
fn fixture_failing_crate() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join("Cargo.toml"),
        "[package]\nname = \"vf\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&dir.path().join(".gitignore"), "target/\nCargo.lock\n");
    write(
        &dir.path().join("src/lib.rs"),
        "pub fn f() -> i32 { 1 }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn always_fails() { panic!(\"passed and still failed\") }\n}\n",
    );
    dir
}

async fn run_testing(
    harness: TestingHarness,
    task: &str,
    root: &Path,
    limits: Option<TestingLimits>,
    command_timeout_secs: u64,
) -> TestingResult {
    let (_, emit) = event_sink();
    let mut subagent = harness.subagent(root, command_timeout_secs);
    let mut request = TestingRequest::new(task, root);
    if let Some(limits) = limits {
        request = request.with_limits(limits);
    }
    subagent.run(request, &emit, None).await
}

// =========================================================================
// Execution: real command execution + authoritative exit code
// =========================================================================

#[tokio::test]
async fn test_testing_executes_real_command_and_captures_exit_code() {
    let dir = fixture_passing_crate();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="run_command">{"command": "cargo check"}</invoke>"#.to_string(),
            "cargo check passed.".to_string(),
        ],
    )));
    let result = run_testing(harness, "validate the crate", dir.path(), None, 60).await;

    assert_eq!(result.termination, TestingTermination::Completed);
    assert!(
        !result.commands_run.is_empty(),
        "testing must run at least one command"
    );
    let check = &result.commands_run[0];
    assert_eq!(check.command, "cargo check");
    assert_eq!(
        check.exit_code, 0,
        "cargo check on a valid crate must exit 0"
    );
    assert!(check.success, "exit 0 must be success");
    assert!(
        result.failures.is_empty(),
        "a passing cargo check must produce no failures"
    );
    assert!(
        !check.output.trim().is_empty(),
        "real command output must be captured"
    );
}

/// The critical Sprint 30D invariant: output containing "passed" with a
/// non-zero exit code is a FAILURE. `cargo test` on the failing crate prints
/// "0 passed; 1 failed" but exits 101.
#[tokio::test]
async fn test_testing_exit_code_authoritative_over_passed_prose() {
    let dir = fixture_failing_crate();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="run_command">{"command": "cargo test"}</invoke>"#.to_string(),
            "The tests look successful to me.".to_string(),
        ],
    )));
    let result = run_testing(harness, "run the tests", dir.path(), None, 60).await;

    assert_eq!(result.termination, TestingTermination::Completed);
    let record = result
        .commands_run
        .first()
        .expect("a command must have run");
    assert_eq!(record.exit_code, 101, "a failing cargo test exits 101");
    assert!(
        record.output.contains("passed"),
        "the fixture's failure output must contain 'passed' (its panic message)"
    );
    assert!(
        !record.success,
        "exit code 101 must be a failure even though the output says 'passed'"
    );
    assert!(
        result
            .failures
            .iter()
            .any(|f| f.kind.as_str() == "test_failure"),
        "the failure must be classified as a test failure"
    );
    // The model's prose ("looks successful") must not leak into the machine
    // facts: the command record stays a failure.
    assert!(
        !result.summary.contains("exit_code: 0"),
        "no fabricated success in the summary"
    );
}

/// Output containing "failed" with exit code 0 is a SUCCESS.
#[tokio::test]
async fn test_testing_exit_zero_with_failed_prose_is_success() {
    let dir = fixture_passing_crate();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="run_command">{"command": "cargo test"}</invoke>"#.to_string(),
            "Final report: validation complete.".to_string(),
        ],
    )));
    let result = run_testing(harness, "run the tests", dir.path(), None, 60).await;

    let record = result
        .commands_run
        .first()
        .expect("a command must have run");
    assert_eq!(record.exit_code, 0);
    assert!(
        record.output.contains("failed_lol"),
        "the passing test named 'failed_lol' prints 'failed' in its output"
    );
    assert!(
        record.success,
        "exit code 0 must be success even though the output contains 'failed'"
    );
    assert!(result.failures.is_empty());
}

// =========================================================================
// Multi-step: command results affect the next iteration
// =========================================================================

#[tokio::test]
async fn test_testing_multi_step_results_feed_next_iteration() {
    let dir = fixture_passing_crate();
    let provider = Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="run_command">{"command": "cargo check"}</invoke>"#.to_string(),
            r#"<invoke name="run_command">{"command": "cargo test"}</invoke>"#.to_string(),
            "Final testing report: cargo check and cargo test both pass.".to_string(),
        ],
    ));
    let harness = TestingHarness::new(provider.clone());
    let limits = TestingLimits {
        max_model_calls: 3,
        reserved_synthesis_calls: 1,
        ..TestingLimits::default()
    };
    let result = run_testing(harness, "validate the crate", dir.path(), Some(limits), 60).await;

    assert_eq!(result.termination, TestingTermination::Completed);
    assert_eq!(result.iterations, 3);
    assert_eq!(result.tool_calls, 2);
    assert_eq!(result.commands_run.len(), 2);
    assert!(result.synthesis_complete);
    assert!(result.commands_run[0].success);
    assert!(result.commands_run[1].success);

    // The evidence trail flowed into the next model calls: iteration 2's
    // prompt contains the cargo check result; iteration 3's (forced reserved
    // synthesis) prompt contains the cargo test result.
    let prompts = provider.prompt_log();
    assert_eq!(prompts.len(), 3, "one prompt per iteration");
    assert!(
        prompts[1].contains("cargo check"),
        "iteration 2 prompt must contain the cargo check result:\n{}",
        prompts[1]
    );
    assert!(
        prompts[1].contains("exit_code: 0"),
        "iteration 2 prompt must contain the authoritative exit code:\n{}",
        prompts[1]
    );
    assert!(
        prompts[2].contains("cargo test"),
        "iteration 3 prompt must contain the cargo test result:\n{}",
        prompts[2]
    );
    assert!(
        prompts[2].contains("FINAL TESTING REPORT"),
        "the last call must be the reserved synthesis prompt:\n{}",
        prompts[2]
    );
}

#[tokio::test]
async fn test_testing_structured_envelope_unwrapped_before_execution() {
    // Real OpenAI-compatible providers wrap tool arguments in the canonical
    // `{"input": "<args>"}` envelope. Testing must unwrap it so the policy
    // sees the raw command and the PTY runs the real command.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::structured(
        "testing-fc",
        vec![
            r#"[{"id": "c1", "function": {"name": "run_command", "arguments": "{\"input\": \"false\"}"}}]"#
                .to_string(),
            "[]".to_string(),
        ],
    )));
    let result = run_testing(harness, "run a check", dir.path(), None, 60).await;

    assert_eq!(result.termination, TestingTermination::Completed);
    let record = result
        .commands_run
        .first()
        .expect("a command must have run");
    assert_eq!(record.exit_code, 1, "false must exit 1");
    assert!(!record.success);
}

// =========================================================================
// Permission: mutating tools and destructive commands are blocked
// =========================================================================

#[tokio::test]
async fn test_testing_denies_destructive_command() {
    let dir = fixture_workspace();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="run_command">{"command": "rm -rf /"}</invoke>"#.to_string(),
            "I tried to run rm but it was denied.".to_string(),
        ],
    )));
    let result = run_testing(harness, "run a cleanup", dir.path(), None, 60).await;

    let record = result
        .commands_run
        .first()
        .expect("the attempted command must be recorded");
    assert!(record.denied, "rm must be denied, got: {:?}", record);
    assert!(!record.success);
    assert_eq!(record.exit_code, -1);
    // The workspace still exists and its files are intact.
    assert!(dir.path().join("src/lib.rs").exists());
    assert!(result
        .observations
        .iter()
        .any(|o| o.result.contains("DENIED") || o.result.contains("denied")));
}

#[tokio::test]
async fn test_testing_denies_git_mutation() {
    let dir = fixture_workspace();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="run_command">{"command": "git commit -m \"boom\""}</invoke>"#
                .to_string(),
            "Git mutation was blocked.".to_string(),
        ],
    )));
    let result = run_testing(harness, "commit changes", dir.path(), None, 60).await;

    let record = result
        .commands_run
        .first()
        .expect("the attempted command must be recorded");
    assert!(record.denied, "git commit must be denied");
    assert!(
        record
            .denied_reason
            .as_deref()
            .unwrap_or("")
            .contains("read-only")
            || record
                .denied_reason
                .as_deref()
                .unwrap_or("")
                .contains("not allowed")
    );
}

#[tokio::test]
async fn test_testing_cannot_create_files() {
    let dir = fixture_workspace();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="create_file">{"path": "src/evil.rs", "content": "boom"}</invoke>"#
                .to_string(),
            "create_file was blocked.".to_string(),
        ],
    )));
    let result = run_testing(harness, "write a file", dir.path(), None, 60).await;

    // The attempt is recorded as a failed observation (never silently run).
    let attempt = result.observations.iter().find(|o| o.name == "create_file");
    assert!(attempt.is_some(), "create_file attempt must be recorded");
    assert!(
        attempt.unwrap().result.starts_with("Error"),
        "create_file must be rejected, got: {}",
        attempt.unwrap().result
    );
    assert!(
        !dir.path().join("src/evil.rs").exists(),
        "testing must never create files"
    );
}

#[tokio::test]
async fn test_testing_denies_mutating_shell_programs() {
    let dir = fixture_workspace();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="run_command">{"command": "python3 -c 'open(\"pwn\",\"w\")'"}</invoke>"#
                .to_string(),
            "python was denied.".to_string(),
        ],
    )));
    let result = run_testing(harness, "run a script", dir.path(), None, 60).await;

    let record = result
        .commands_run
        .first()
        .expect("the attempted command must be recorded");
    assert!(record.denied, "python3 must be denied");
    assert!(!dir.path().join("pwn").exists());
}

// =========================================================================
// Command timeout
// =========================================================================

#[tokio::test]
async fn test_testing_command_timeout_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="run_command">{"command": "sleep 3"}</invoke>"#.to_string(),
            "Final report: the command was cut off by its timeout.".to_string(),
        ],
    )));
    // 1s per-command PTY timeout against a 3s sleep: deterministic, bounded,
    // and NOT an infinite process.
    let result = run_testing(harness, "run a slow command", dir.path(), None, 1).await;

    let record = result
        .commands_run
        .first()
        .expect("a command must have run");
    assert!(
        record.timeout,
        "sleep 3 must time out under a 1s per-command timeout"
    );
    assert!(!record.success);
    assert_eq!(record.exit_code, -1);
    assert!(
        result.failures.iter().any(|f| f.kind.as_str() == "timeout"),
        "the timeout must be recorded as a timeout failure"
    );
}

// =========================================================================
// Bounds
// =========================================================================

#[tokio::test]
async fn test_testing_iteration_limit() {
    let dir = fixture_workspace();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="run_command">{"command": "true"}</invoke>"#.to_string(),
            r#"<invoke name="run_command">{"command": "true"}</invoke>"#.to_string(),
            r#"<invoke name="run_command">{"command": "true"}</invoke>"#.to_string(),
        ],
    )));
    let limits = TestingLimits {
        max_iterations: 1,
        ..TestingLimits::default()
    };
    let result = run_testing(harness, "run forever", dir.path(), Some(limits), 30).await;

    assert_eq!(result.termination, TestingTermination::IterationLimit);
    assert!(result.iterations <= 1);
}

#[tokio::test]
async fn test_testing_tool_call_limit() {
    let dir = fixture_workspace();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![r#"<invoke name="run_command">{"command": "true"}</invoke>
<invoke name="run_command">{"command": "false"}</invoke>"#
            .to_string()],
    )));
    let limits = TestingLimits {
        max_tool_calls: 1,
        ..TestingLimits::default()
    };
    let result = run_testing(harness, "run two commands", dir.path(), Some(limits), 30).await;

    assert_eq!(result.termination, TestingTermination::ToolLimit);
    assert!(result.tool_calls <= 1);
}

#[tokio::test]
async fn test_testing_model_call_limit() {
    let dir = fixture_workspace();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="run_command">{"command": "true"}</invoke>"#.to_string(),
            r#"<invoke name="run_command">{"command": "true"}</invoke>"#.to_string(),
        ],
    )));
    let limits = TestingLimits {
        max_model_calls: 1,
        ..TestingLimits::default()
    };
    let result = run_testing(harness, "run once", dir.path(), Some(limits), 30).await;

    assert_eq!(result.termination, TestingTermination::ModelLimit);
    assert!(result.model_calls <= 1);
    // No synthesis is fabricated when the model budget is exhausted before it.
    assert!(!result.synthesis_complete);
    assert!(result.limitations.iter().any(|l| l.contains("model-call")));
}

#[tokio::test]
async fn test_testing_timeout_terminates() {
    let dir = fixture_workspace();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec!["I never finish this.".to_string()],
    )));
    // A zero-time budget makes the timeout deterministic: the loop's deadline
    // check fires before any provider call.
    let limits = TestingLimits {
        timeout_ms: 0,
        ..TestingLimits::default()
    };
    let result = run_testing(harness, "test slowly", dir.path(), Some(limits), 30).await;

    assert_eq!(result.termination, TestingTermination::Timeout);
    assert_eq!(result.iterations, 0);
}

#[tokio::test]
async fn test_testing_cancellation() {
    let dir = fixture_workspace();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec!["I am slow.".to_string()],
    )));
    let (_, emit) = event_sink();
    let token = crate::cancellation::CancellationToken::new();
    token.cancel();
    let mut subagent = harness.subagent(dir.path(), 30);
    let request = TestingRequest::new("testing cancelled", dir.path());
    let result = subagent.run(request, &emit, Some(token)).await;

    assert_eq!(result.termination, TestingTermination::Cancelled);
    assert!(result.limitations.iter().any(|l| l.contains("cancel")));
}

// =========================================================================
// Failure isolation
// =========================================================================

#[tokio::test]
async fn test_testing_provider_failure_is_bounded_error() {
    let dir = fixture_workspace();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::failing("testing-fail")));
    let result = run_testing(harness, "validate the crate", dir.path(), None, 30).await;

    assert_eq!(result.termination, TestingTermination::Error);
    assert!(result
        .limitations
        .iter()
        .any(|l| l.contains("testing mock provider offline")));
    assert!(result.render().contains("Autonomous Testing Findings"));
    assert_eq!(result.commands_run.len(), 0);
}

// =========================================================================
// No mutation
// =========================================================================

fn run_git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git must run");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn init_git_repo(root: &Path) {
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(root)
        .output()
        .unwrap()
        .status
        .success());
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "test@test"])
        .current_dir(root)
        .output();
    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(root)
        .output();
    assert!(std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(std::process::Command::new("git")
        .args(["commit", "-q", "-m", "initial"])
        .current_dir(root)
        .output()
        .unwrap()
        .status
        .success());
}

/// Testing must not mutate tracked source or git state. Normal ignored build
/// artifacts (`target/`) are allowed. The model attempts a valid validation
/// command plus mutating actions that must all be denied.
#[tokio::test]
async fn test_testing_never_mutates_repository_state() {
    let dir = fixture_passing_crate();
    init_git_repo(dir.path());

    let status_before = run_git(dir.path(), &["status", "--short"]);
    let diff_before = run_git(dir.path(), &["diff"]);
    let lib_before = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();

    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="run_command">{"command": "cargo check"}</invoke>"#.to_string(),
            r#"<invoke name="create_file">{"path": "src/evil.rs", "content": "fn evil() {}"}</invoke>"#
                .to_string(),
            r#"<invoke name="run_command">{"command": "rm -f src/lib.rs"}</invoke>"#.to_string(),
            "Testing complete.".to_string(),
        ],
    )));
    let result = run_testing(harness, "validate the crate", dir.path(), None, 60).await;

    assert_eq!(result.termination, TestingTermination::Completed);
    assert!(
        result.git_tree_unchanged(),
        "git state must be unchanged after testing"
    );

    let status_after = run_git(dir.path(), &["status", "--short"]);
    let diff_after = run_git(dir.path(), &["diff"]);
    assert_eq!(
        status_before, status_after,
        "git status must be identical before and after"
    );
    assert_eq!(
        diff_before, diff_after,
        "git diff must be identical before and after"
    );
    let lib_after = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert_eq!(lib_before, lib_after, "tracked source must be unchanged");
    assert!(
        !dir.path().join("src/evil.rs").exists(),
        "the attempted create_file must not have created anything"
    );
    assert!(
        lib_after.contains("pub fn add"),
        "the attempted rm must not have deleted the tracked file"
    );

    // The rm attempt was denied; the cargo check ran and built target/ (an
    // allowed ignored build artifact).
    assert!(
        result
            .commands_run
            .iter()
            .any(|c| c.command == "cargo check" && c.success),
        "cargo check must have run successfully"
    );
    assert!(
        result
            .commands_run
            .iter()
            .any(|c| c.command == "rm -f src/lib.rs" && c.denied),
        "rm must have been denied"
    );
    // target/ may exist — normal compiler artifacts are acceptable.
    let _ = dir.path().join("target");
}

// =========================================================================
// Observability / performance
// =========================================================================

/// Observational testing performance report. Ignored by default; run with
/// `cargo test --bin codebro testing::tests::testing_performance_report
/// -- --ignored --nocapture`. Numbers are machine-dependent.
#[tokio::test]
#[ignore]
async fn testing_performance_report() {
    let dir = fixture_passing_crate();
    let harness = TestingHarness::new(Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="run_command">{"command": "cargo check"}</invoke>"#.to_string(),
            r#"<invoke name="run_command">{"command": "cargo test"}</invoke>"#.to_string(),
            "All validation commands pass.".to_string(),
        ],
    )));
    let result = run_testing(harness, "validate the crate", dir.path(), None, 60).await;

    println!(
        "\n[testing-perf] termination={} iterations={} tool_calls={} model_calls={} commands={} failures={} files={} duration={}ms output={}B",
        result.termination,
        result.iterations,
        result.tool_calls,
        result.model_calls,
        result.commands_run.len(),
        result.failures.len(),
        result.files_inspected.len(),
        result.duration_ms,
        result.output_size,
    );
    println!("[testing-perf] rendered_bytes={}", result.render().len());
    for command in &result.commands_run {
        println!(
            "[testing-perf] command={} exit_code={} success={} duration_ms={} output_chars={}",
            command.command,
            command.exit_code,
            command.success,
            command.duration_ms,
            command.output.chars().count()
        );
    }
    assert_eq!(result.termination, TestingTermination::Completed);
    assert!(result.commands_run.iter().all(|c| c.success));
}

/// Deterministic before/after evidence trace (Sprint 30D §21). Prints the
/// actual objective → command → authoritative exit code → next-decision loop.
/// Run with `cargo test --bin codebro testing::tests::testing_before_after_trace
/// -- --ignored --nocapture`.
#[tokio::test]
#[ignore]
async fn testing_before_after_trace() {
    let dir = fixture_passing_crate();
    let provider = Arc::new(TestingMockProvider::text(
        "testing-mock",
        vec![
            r#"<invoke name="run_command">{"command": "cargo check"}</invoke>"#.to_string(),
            r#"<invoke name="run_command">{"command": "cargo test"}</invoke>"#.to_string(),
            "Validation complete: cargo check and cargo test pass.".to_string(),
        ],
    ));
    let harness = TestingHarness::new(provider.clone());
    let result = run_testing(harness, "validate the crate", dir.path(), None, 60).await;

    println!("\n===== SPRINT 30D AUTONOMOUS TESTING TRACE =====");
    println!("Objective: validate the crate");
    let prompts = provider.prompt_log();
    for (i, prompt) in prompts.iter().enumerate() {
        println!("\n--- model call {} ---", i + 1);
        if let Some(idx) = prompt.find("PREVIOUS COMMAND RESULTS:") {
            let rest = &prompt[idx..];
            let head: String = rest.lines().take(8).collect::<Vec<_>>().join("\n");
            println!("{}", head);
        }
    }
    println!("\n--- TestingResult ---");
    println!("termination: {}", result.termination);
    println!("iterations: {}", result.iterations);
    println!("commands run: {}", result.commands_run.len());
    for command in &result.commands_run {
        println!(
            "  $ {} -> exit_code={} success={}",
            command.command, command.exit_code, command.success
        );
    }
    println!("=============================\n");
    assert_eq!(result.termination, TestingTermination::Completed);
}
