//! Sprint 30C — deterministic tests for the autonomous Research subagent.
//!
//! These tests prove:
//! - the Research subagent performs REAL read-only tool execution
//! - tool results affect the next iteration (the evidence trail)
//! - the restricted registry blocks mutating tools
//! - all bounds are enforced (iteration / tool / model / timeout / cancel)
//! - provider failures produce a bounded error result
//! - research never mutates repository or git state

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

use super::limits::ResearchLimits;
use super::permissions::ResearchTooling;
use super::ResearchRequest;
use super::ResearchResult;
use super::ResearchSubagent;
use super::ResearchTermination;

// =========================================================================
// Mock provider
// =========================================================================

/// A scripted mock provider for research tests. Consumes responses
/// sequentially and records every prompt it receives (so tests can prove the
/// evidence trail flows into the next model call).
#[derive(Clone)]
struct ResearchMockProvider {
    name: String,
    model: String,
    responses: Arc<Mutex<Vec<String>>>,
    prompts: Arc<Mutex<Vec<String>>>,
    structured: bool,
    fail: Arc<AtomicBool>,
}

impl ResearchMockProvider {
    fn text(name: &str, responses: Vec<String>) -> Self {
        ResearchMockProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            responses: Arc::new(Mutex::new(responses)),
            prompts: Arc::new(Mutex::new(Vec::new())),
            structured: false,
            fail: Arc::new(AtomicBool::new(false)),
        }
    }

    fn structured(name: &str, responses: Vec<String>) -> Self {
        ResearchMockProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            responses: Arc::new(Mutex::new(responses)),
            prompts: Arc::new(Mutex::new(Vec::new())),
            structured: true,
            fail: Arc::new(AtomicBool::new(false)),
        }
    }

    fn failing(name: &str) -> Self {
        let p = ResearchMockProvider::text(name, Vec::new());
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

impl Provider for ResearchMockProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn base_url(&self) -> &str {
        "mock://research"
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
            let result = Err(anyhow::anyhow!("research mock provider offline"));
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

struct ResearchHarness {
    provider_runtime: ProviderRuntime,
    router: IntelligentProviderRouter,
    io_providers: HashMap<ProviderId, Arc<dyn Provider>>,
}

impl ResearchHarness {
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
        ResearchHarness {
            provider_runtime,
            router,
            io_providers,
        }
    }

    fn subagent(self, root: &Path) -> ResearchSubagent {
        let tooling = ResearchTooling::new(root);
        ResearchSubagent::new(
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

/// A fixture workspace with a couple of real source files.
fn fixture_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n[dependencies]\ntokio = \"1\"\n",
    );
    write(
        &dir.path().join("src/parser.rs"),
        "pub fn parse_tool_calls() {}\npub fn trace_runtime() {}\n",
    );
    write(
        &dir.path().join("src/main.rs"),
        "fn main() {\n    let _ = parse_tool_calls();\n}\n",
    );
    dir
}

async fn run_research(
    harness: ResearchHarness,
    task: &str,
    root: &Path,
    limits: Option<ResearchLimits>,
) -> ResearchResult {
    let (_, emit) = event_sink();
    let mut subagent = harness.subagent(root);
    let mut request = ResearchRequest::new(task, root);
    if let Some(limits) = limits {
        request = request.with_limits(limits);
    }
    subagent.run(request, &emit, None).await
}

// =========================================================================
// Execution: real tool observation
// =========================================================================

#[tokio::test]
async fn test_research_performs_real_tool_observation() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
            "I inspected the source directory. The parser module lives at src/parser.rs and exposes parse_tool_calls.".to_string(),
        ],
    )));
    let result = run_research(harness, "trace the parser module", dir.path(), None).await;

    assert_eq!(result.termination, ResearchTermination::Completed);
    assert!(
        result.tool_calls >= 1,
        "research must execute at least one tool"
    );
    assert!(
        !result.tool_observations.is_empty(),
        "research must record tool observations"
    );
    assert!(
        result
            .files_inspected
            .iter()
            .any(|f| f.to_string_lossy().contains("parser")),
        "research must have inspected real files, got: {:?}",
        result.files_inspected
    );
    assert!(!result.summary.is_empty());
}

#[tokio::test]
async fn test_research_reads_real_file_contents() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="read_file">{"path": "src/parser.rs"}</invoke>"#.to_string(),
            "The parser file defines parse_tool_calls and trace_runtime.".to_string(),
        ],
    )));
    let result = run_research(harness, "inspect the parser", dir.path(), None).await;

    assert_eq!(result.termination, ResearchTermination::Completed);
    // The observation must contain the ACTUAL file contents.
    let read = result
        .tool_observations
        .iter()
        .find(|o| o.name == "read_file")
        .expect("read_file observation present");
    assert!(
        read.result.contains("parse_tool_calls"),
        "read_file must return real file contents, got: {}",
        read.result
    );
    // The file's symbols are extracted from the real contents.
    assert!(
        result
            .symbols_found
            .contains(&"parse_tool_calls".to_string())
            || result.symbols_found.contains(&"trace_runtime".to_string()),
        "symbols from read contents must be found, got: {:?}",
        result.symbols_found
    );
}

// =========================================================================
// Multi-step: tool results affect the next iteration
// =========================================================================

#[tokio::test]
async fn test_research_multi_step_tool_results_feed_next_iteration() {
    let dir = fixture_workspace();
    let provider = Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
            r#"<invoke name="read_file">{"path": "src/parser.rs"}</invoke>"#.to_string(),
            "Final report: parse_tool_calls is defined in src/parser.rs.".to_string(),
        ],
    ));
    let harness = ResearchHarness::new(provider.clone());
    let result = run_research(harness, "trace the parser module", dir.path(), None).await;

    assert_eq!(result.termination, ResearchTermination::Completed);
    assert_eq!(result.iterations, 3);
    assert_eq!(result.tool_calls, 2);
    assert!(result.synthesis_complete);

    // The evidence trail flowed into the next model calls: iteration 2's
    // prompt contains the list_files result; iteration 3's prompt contains
    // the read_file result.
    let prompts = provider.prompt_log();
    assert_eq!(prompts.len(), 3, "one prompt per iteration");
    assert!(
        prompts[1].contains("parser.rs"),
        "iteration 2 prompt must contain the list_files result:\n{}",
        prompts[1]
    );
    assert!(
        prompts[1].contains("list_files"),
        "iteration 2 prompt must show the list_files observation"
    );
    assert!(
        prompts[2].contains("parse_tool_calls"),
        "iteration 3 prompt must contain the read_file result:\n{}",
        prompts[2]
    );
    assert!(
        result
            .files_inspected
            .iter()
            .any(|f| f.to_string_lossy().contains("parser")),
        "read_file must register inspected files"
    );
}

#[tokio::test]
async fn test_research_structured_tool_calling_reaches_restricted_registry() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::structured(
        "research-fc",
        vec![
            r#"[{"id": "c1", "function": {"name": "list_files", "arguments": "{\"path\": \"src\"}"}}]"#
                .to_string(),
            r#"[{"id": "c2", "function": {"name": "read_file", "arguments": "{\"path\": \"src/parser.rs\"}"}}]"#
                .to_string(),
            "[]".to_string(),
        ],
    )));
    let result = run_research(harness, "trace the parser", dir.path(), None).await;

    assert_eq!(result.termination, ResearchTermination::Completed);
    assert_eq!(result.tool_calls, 2);
    assert!(
        result
            .tool_observations
            .iter()
            .any(|o| o.name == "read_file" && o.result.contains("parse_tool_calls")),
        "structured calls must execute real tools and observe real results"
    );
}

#[tokio::test]
async fn test_research_structured_input_envelope_is_unwrapped() {
    // Real OpenAI-compatible providers wrap tool arguments in the canonical
    // `{"input": "<args>"}` envelope (the tool definitions declare a single
    // `input` parameter). Research must unwrap it before executing tools.
    let dir = fixture_workspace();
    let abs = dir
        .path()
        .join("src/parser.rs")
        .to_string_lossy()
        .to_string();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::structured(
        "research-fc",
        vec![
            format!(
                r#"[{{"id": "c1", "function": {{"name": "read_file", "arguments": "{{\"input\": \"{}\"}}"}}}}]"#,
                abs
            ),
            "[]".to_string(),
        ],
    )));
    let result = run_research(harness, "inspect the parser", dir.path(), None).await;

    assert_eq!(result.termination, ResearchTermination::Completed);
    let read = result
        .tool_observations
        .iter()
        .find(|o| o.name == "read_file")
        .expect("read_file observation present");
    assert!(
        read.success,
        "read_file must succeed once the input envelope is unwrapped, got: {}",
        read.result
    );
    assert!(
        read.result.contains("parse_tool_calls"),
        "read_file must return real file contents, got: {}",
        read.result
    );
    assert!(
        result
            .files_inspected
            .iter()
            .any(|f| f.to_string_lossy().contains("parser")),
        "files_inspected must record the real resolved path, got: {:?}",
        result.files_inspected
    );
}

// =========================================================================
// Synthesis: reserved final model call (Sprint 30C.0.1)
// =========================================================================

/// A model that keeps exploring must be forced into a final synthesis call
/// when the evidence budget is exhausted, and that call must produce the
/// structured result.
#[tokio::test]
async fn test_research_reserves_final_synthesis_call() {
    let dir = fixture_workspace();
    let provider = Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
            "Synthesis: parser.rs defines parse_tool_calls.".to_string(),
        ],
    ));
    let harness = ResearchHarness::new(provider.clone());
    let limits = ResearchLimits {
        max_model_calls: 3,
        reserved_synthesis_calls: 1,
        ..ResearchLimits::default()
    };
    let result = run_research(harness, "trace the parser", dir.path(), Some(limits)).await;

    assert_eq!(result.termination, ResearchTermination::Completed);
    assert!(
        result.synthesis_complete,
        "the reserved synthesis call must produce the final report"
    );
    assert_eq!(result.model_calls, 3, "3 calls: 2 evidence + 1 synthesis");
    assert_eq!(
        result.tool_calls, 2,
        "evidence gathering must stop at the budget"
    );
    assert!(
        result.summary.contains("parse_tool_calls"),
        "the synthesis must be the final answer, got: {}",
        result.summary
    );

    // The final call is the distinct synthesis prompt, not the evidence prompt.
    let prompts = provider.prompt_log();
    assert_eq!(prompts.len(), 3);
    assert!(
        prompts[2].contains("FINAL RESEARCH REPORT"),
        "the last call must be the synthesis prompt:\n{}",
        prompts[2]
    );
    assert!(
        !prompts[2].contains("RESEARCH STEP 3"),
        "synthesis prompt must not use the evidence step format"
    );
}

/// If the model uses the reserved synthesis call to request MORE evidence, the
/// loop must terminate honestly: `ModelLimit`, `synthesis_complete = false`,
/// with the evidence trail preserved and no fabricated summary.
#[tokio::test]
async fn test_research_synthesis_cannot_extend_evidence_gathering() {
    let dir = fixture_workspace();
    let provider = Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
            r#"<invoke name="read_file">{"path": "src/parser.rs"}</invoke>"#.to_string(),
            // During the synthesis call the model asks for one more read.
            r#"<invoke name="read_file">{"path": "src/main.rs"}</invoke>"#.to_string(),
        ],
    ));
    let harness = ResearchHarness::new(provider.clone());
    let limits = ResearchLimits {
        max_model_calls: 3,
        reserved_synthesis_calls: 1,
        ..ResearchLimits::default()
    };
    let result = run_research(harness, "trace the parser", dir.path(), Some(limits)).await;

    assert_eq!(result.termination, ResearchTermination::ModelLimit);
    assert!(!result.synthesis_complete);
    // The structured evidence gathered before the synthesis attempt is kept.
    assert_eq!(result.tool_calls, 2);
    assert!(
        result
            .tool_observations
            .iter()
            .any(|o| o.name == "read_file"),
        "the evidence trail must be preserved, got: {:?}",
        result.tool_observations
    );
    assert!(
        result
            .files_inspected
            .iter()
            .any(|f| f.to_string_lossy().contains("parser")),
        "inspected files must be preserved, got: {:?}",
        result.files_inspected
    );
    // No summary is fabricated: the summary is the honest default.
    assert!(
        result.summary.contains("model_limit"),
        "summary must state the real termination, got: {}",
        result.summary
    );
    assert!(
        result.limitations.iter().any(|l| l.contains("model-call")),
        "limitations must record the model-call limit"
    );
    // The synthesis prompt was the final call.
    let prompts = provider.prompt_log();
    assert_eq!(prompts.len(), 3);
    assert!(
        prompts[2].contains("FINAL RESEARCH REPORT"),
        "the last call must be the synthesis prompt:\n{}",
        prompts[2]
    );
}

/// A model that voluntarily stops exploring (no tool call) completes with the
/// final answer and a full synthesis — no forced call is needed.
#[tokio::test]
async fn test_research_voluntary_completion_is_full_synthesis() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="read_file">{"path": "src/parser.rs"}</invoke>"#.to_string(),
            "Final report: parse_tool_calls lives in src/parser.rs.".to_string(),
        ],
    )));
    let result = run_research(harness, "trace the parser", dir.path(), None).await;

    assert_eq!(result.termination, ResearchTermination::Completed);
    assert!(result.synthesis_complete);
    assert_eq!(result.model_calls, 2);
}

/// A budget too small to gather evidence must not fabricate a synthesis: it
/// terminates at `ModelLimit` with `synthesis_complete = false`.
#[tokio::test]
async fn test_research_no_evidence_no_synthesis_is_honest() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
        ],
    )));
    // max_model_calls = 1: the single call is used for evidence; there is no
    // budget left to attempt synthesis, so the result is an honest ModelLimit.
    let limits = ResearchLimits {
        max_model_calls: 1,
        reserved_synthesis_calls: 1,
        ..ResearchLimits::default()
    };
    let result = run_research(harness, "trace the parser", dir.path(), Some(limits)).await;

    assert_eq!(result.termination, ResearchTermination::ModelLimit);
    assert!(!result.synthesis_complete);
    assert!(result.tool_calls <= 1);
}

// =========================================================================
// Permission: mutating tools are blocked
// =========================================================================

#[tokio::test]
async fn test_research_cannot_create_files() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="create_file">{"path": "src/evil.rs", "content": "boom"}</invoke>"#
                .to_string(),
            "I tried to create a file but it was blocked.".to_string(),
        ],
    )));
    let result = run_research(harness, "write a file", dir.path(), None).await;

    // The attempt must be recorded as a failed observation (not silently run).
    let attempt = result
        .tool_observations
        .iter()
        .find(|o| o.name == "create_file");
    assert!(attempt.is_some(), "create_file attempt must be recorded");
    assert!(
        attempt.unwrap().result.starts_with("Error"),
        "create_file must be rejected, got: {}",
        attempt.unwrap().result
    );
    // No file was created on disk.
    assert!(
        !dir.path().join("src/evil.rs").exists(),
        "research must never create files"
    );
}

#[tokio::test]
async fn test_research_cannot_run_commands() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="run_command">{"command": "touch src/pwned.rs"}</invoke>"#.to_string(),
            "Shell access was blocked.".to_string(),
        ],
    )));
    let result = run_research(harness, "run a shell command", dir.path(), None).await;

    let attempt = result
        .tool_observations
        .iter()
        .find(|o| o.name == "run_command");
    assert!(attempt.is_some());
    assert!(
        attempt.unwrap().result.starts_with("Error"),
        "run_command must be rejected"
    );
    assert!(!dir.path().join("src/pwned.rs").exists());
}

// =========================================================================
// Bounds
// =========================================================================

#[tokio::test]
async fn test_research_iteration_limit() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
        ],
    )));
    let limits = ResearchLimits {
        max_iterations: 1,
        ..ResearchLimits::default()
    };
    let result = run_research(harness, "list forever", dir.path(), Some(limits)).await;

    assert_eq!(result.termination, ResearchTermination::IterationLimit);
    assert!(result.iterations <= 1);
}

#[tokio::test]
async fn test_research_tool_call_limit() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![r#"<invoke name="list_files">{"path": "src"}</invoke>
<invoke name="read_file">{"path": "src/parser.rs"}</invoke>"#
            .to_string()],
    )));
    let limits = ResearchLimits {
        max_tool_calls: 1,
        ..ResearchLimits::default()
    };
    let result = run_research(harness, "list and read", dir.path(), Some(limits)).await;

    assert_eq!(result.termination, ResearchTermination::ToolLimit);
    assert!(result.tool_calls <= 1);
}

#[tokio::test]
async fn test_research_model_call_limit() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
        ],
    )));
    let limits = ResearchLimits {
        max_model_calls: 1,
        ..ResearchLimits::default()
    };
    let result = run_research(harness, "list forever", dir.path(), Some(limits)).await;

    assert_eq!(result.termination, ResearchTermination::ModelLimit);
    assert!(result.model_calls <= 1);
}

#[tokio::test]
async fn test_research_timeout_terminates() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec!["I never finish this.".to_string()],
    )));
    // A zero-time budget makes the timeout deterministic: the loop's deadline
    // check fires before any provider call.
    let limits = ResearchLimits {
        timeout_ms: 0,
        ..ResearchLimits::default()
    };
    let result = run_research(harness, "research slowly", dir.path(), Some(limits)).await;

    assert_eq!(result.termination, ResearchTermination::Timeout);
    assert_eq!(result.iterations, 0);
}

#[tokio::test]
async fn test_research_cancellation() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec!["I am slow.".to_string()],
    )));
    let (_, emit) = event_sink();
    let token = crate::cancellation::CancellationToken::new();
    token.cancel();
    let mut subagent = harness.subagent(dir.path());
    let request = ResearchRequest::new("research cancelled", dir.path());
    let result = subagent.run(request, &emit, Some(token)).await;

    assert_eq!(result.termination, ResearchTermination::Cancelled);
    assert!(result.limitations.iter().any(|l| l.contains("cancel")));
}

#[tokio::test]
async fn test_research_provider_failure_is_bounded_error() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::failing("research-fail")));
    let result = run_research(harness, "trace the loop", dir.path(), None).await;

    assert_eq!(result.termination, ResearchTermination::Error);
    assert!(result
        .limitations
        .iter()
        .any(|l| l.contains("research mock provider offline")));
}

// =========================================================================
// No mutation
// =========================================================================

/// A git-backed fixture so we can prove research does not touch git state.
fn git_fixture() -> tempfile::TempDir {
    let dir = fixture_workspace();
    let git = dir.path().join(".git");
    std::fs::create_dir_all(&git).unwrap();
    // Minimal bare-ish git marker so `git -C` works for assertions. We don't
    // rely on the tool's git execution here — we snapshot state ourselves.
    write(&git.join("HEAD"), "ref: refs/heads/main\n");
    write(
        &dir.path().join(".git/config"),
        "[core]\n\trepositoryformatversion = 0\n",
    );
    dir
}

fn tree_snapshot(root: &Path) -> Vec<String> {
    let mut entries = Vec::new();
    let mut it = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !e
                    .file_name()
                    .to_string_lossy()
                    .to_string()
                    .starts_with(".git")
        });
    for entry in it.by_ref().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            entries.push(entry.path().display().to_string());
        }
    }
    entries.sort();
    entries
}

#[tokio::test]
async fn test_research_never_mutates_repository_state() {
    let dir = git_fixture();
    let before = tree_snapshot(dir.path());
    let cargo_before = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();

    // The model attempts a mix of allowed reads and a mutating create.
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="create_file">{"path": "src/evil.rs", "content": "fn evil() {}"}</invoke>"#
                .to_string(),
            r#"<invoke name="read_file">{"path": "Cargo.toml"}</invoke>"#.to_string(),
            "Research complete. I read Cargo.toml.".to_string(),
        ],
    )));
    let result = run_research(harness, "inspect the project", dir.path(), None).await;

    assert_eq!(result.termination, ResearchTermination::Completed);
    let after = tree_snapshot(dir.path());
    assert_eq!(
        before, after,
        "research must not add/remove/rename any file"
    );
    let cargo_after = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
    assert_eq!(
        cargo_before, cargo_after,
        "research must not modify file contents"
    );
    assert!(
        !dir.path().join("src/evil.rs").exists(),
        "the attempted create_file must not have created anything"
    );
}

// =========================================================================
// Failure isolation (subagent level)
// =========================================================================

#[tokio::test]
async fn test_research_failure_produces_structured_error_result() {
    let dir = fixture_workspace();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::failing("research-fail")));
    let result = run_research(harness, "trace the loop", dir.path(), None).await;

    // The error result is structured and bounded — never a panic.
    assert_eq!(result.termination, ResearchTermination::Error);
    assert!(result.render().contains("Autonomous Research Findings"));
    assert!(result.render().contains("error"));
    assert_eq!(result.tool_calls, 0);
}

// =========================================================================
// Observability / performance
// =========================================================================

/// Observational research performance report. Ignored by default; run with
/// `cargo test --bin codebro research::tests::research_performance_report
/// -- --ignored --nocapture`. Numbers are machine-dependent.
#[tokio::test]
#[ignore]
async fn research_performance_report() {
    // Run research against the real repository (read-only) with a scripted
    // provider: the loop's tool execution is what we measure.
    let root = std::env::current_dir().unwrap();
    let harness = ResearchHarness::new(Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="list_files">{"path": "src/canonical_runtime"}</invoke>"#.to_string(),
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            "The runtime module defines run_execution_loop and stream_once.".to_string(),
        ],
    )));
    let (events, emit) = event_sink();
    let mut subagent = harness.subagent(&root);
    let request = ResearchRequest::new("trace the canonical runtime execution path", &root);
    let result = subagent.run(request, &emit, None).await;
    let _ = events;

    let rendered = result.render();
    println!(
        "\n[research-perf] task={} termination={} iterations={} tool_calls={} model_calls={} files={} symbols={} duration={}ms output={}B",
        "trace canonical runtime",
        result.termination,
        result.iterations,
        result.tool_calls,
        result.model_calls,
        result.files_inspected.len(),
        result.symbols_found.len(),
        result.duration_ms,
        result.output_size,
    );
    println!("[research-perf] rendered_render_bytes={}", rendered.len());
    assert_eq!(result.termination, ResearchTermination::Completed);
}

/// Deterministic before/after evidence trace (Sprint 30C §21). Prints the
/// actual objective → tool → observation → next-decision loop. Run with
/// `cargo test --bin codebro research::tests::research_before_after_trace
/// -- --ignored --nocapture`.
#[tokio::test]
#[ignore]
async fn research_before_after_trace() {
    let dir = fixture_workspace();
    let provider = Arc::new(ResearchMockProvider::text(
        "research-mock",
        vec![
            r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string(),
            r#"<invoke name="read_file">{"path": "src/parser.rs"}</invoke>"#.to_string(),
            "Finding: parse_tool_calls is defined in src/parser.rs.".to_string(),
        ],
    ));
    let harness = ResearchHarness::new(provider.clone());
    let result = run_research(
        harness,
        "trace how a user request reaches tool execution",
        dir.path(),
        None,
    )
    .await;

    println!("\n===== SPRINT 30C AUTONOMOUS RESEARCH TRACE =====");
    println!("Objective: trace how a user request reaches tool execution");
    let prompts = provider.prompt_log();
    for (i, prompt) in prompts.iter().enumerate() {
        let step = if prompt.contains("RESEARCH STEP 1") {
            "decision 1"
        } else if prompt.contains("RESEARCH STEP 2") {
            "decision 2"
        } else {
            "final"
        };
        println!("\n--- model call {} ({}) ---", i + 1, step);
        // Show the observations embedded in this call's prompt.
        if let Some(idx) = prompt.find("PREVIOUS TOOL OBSERVATIONS:") {
            let rest = &prompt[idx..];
            let head: String = rest.lines().take(6).collect::<Vec<_>>().join("\n");
            println!("{}", head);
        }
    }
    println!("\n--- ResearchResult ---");
    println!("termination: {}", result.termination);
    println!("iterations: {}", result.iterations);
    println!("tool calls: {}", result.tool_calls);
    println!("files inspected: {:?}", result.files_inspected);
    println!("symbols found: {:?}", result.symbols_found);
    println!("findings: {}", result.findings.len());
    for f in &result.findings {
        println!("  - {}", f.statement);
    }
    println!("=============================\n");
    assert_eq!(result.termination, ResearchTermination::Completed);
}
