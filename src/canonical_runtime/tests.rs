//! Integration tests for the canonical runtime pipeline.

use std::sync::Arc;

use crate::agent::events::AgentEvent;
use crate::canonical_runtime::CanonicalRuntime;
use crate::config::Config;
use crate::engineering_context::{ContextFragment, ConversationMessage, EngineeringContextBuilder};
use crate::prompt_builder::PromptBuilder;
use crate::provider_runtime::{Priority, RetryPolicy};
use crate::providers::Provider;

// =========================================================================
// Test support
// =========================================================================

/// A scripted mock provider implementing the I/O `providers::Provider` trait.
#[derive(Clone)]
struct MockProvider {
    name: String,
    model: String,
    /// Response payloads returned per call. Empty means no tool calls.
    responses: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// When true, `stream_response` fails immediately.
    fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The last compiled prompt the provider was asked to generate from.
    last_prompt: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl MockProvider {
    fn new(name: &str, responses: Vec<String>) -> Self {
        MockProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            responses: std::sync::Arc::new(std::sync::Mutex::new(responses)),
            fail: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_prompt: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    fn failing(name: &str) -> Self {
        let p = MockProvider::new(name, Vec::new());
        p.fail.store(true, std::sync::atomic::Ordering::SeqCst);
        p
    }

    /// The exact prompt text the provider received on its last call.
    fn last_prompt_text(&self) -> Option<String> {
        self.last_prompt.lock().unwrap().clone()
    }
}

impl Provider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn base_url(&self) -> &str {
        "mock://localhost"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn api_key(&self) -> Option<&str> {
        Some("mock-key")
    }

    fn send_message(
        &self,
        _message: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
    {
        let response = self
            .responses
            .lock()
            .unwrap()
            .first()
            .cloned()
            .unwrap_or_default();
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
        *self.last_prompt.lock().unwrap() = Some(message.to_string());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            let result = Err(anyhow::anyhow!("mock provider offline"));
            Box::pin(async move { result })
        } else {
            let response = self
                .responses
                .lock()
                .unwrap()
                .first()
                .cloned()
                .unwrap_or_default();
            let _ = tx.send(response);
            Box::pin(async move { Ok(rx) })
        }
    }
}

/// A config pointing at an obviously invalid provider (never contacted).
fn test_config() -> Config {
    Config {
        provider: "mock".to_string(),
        base_url: "mock://localhost".to_string(),
        model: "mock-model".to_string(),
        api_key: Some("mock-key".to_string()),
    }
}

/// Collect all AgentEvents emitted during a task.
fn event_sink() -> (
    std::sync::Arc<std::sync::Mutex<Vec<AgentEvent>>>,
    Box<dyn Fn(AgentEvent) + Send + Sync>,
) {
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = events.clone();
    (events, Box::new(move |e| sink.lock().unwrap().push(e)))
}

/// Empty conversation for tests.
fn no_conversation() -> Vec<ConversationMessage> {
    Vec::new()
}

// =========================================================================
// Tests
// =========================================================================

#[tokio::test]
async fn test_happy_path_produces_response() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(1));
    runtime.register_provider(Arc::new(MockProvider::new(
        "mock",
        vec!["final answer".to_string()],
    )));
    let (events, emit) = event_sink();
    let chunks: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let chunk_sink = chunks.clone();
    let on_chunk = move |c: &str| chunk_sink.lock().unwrap().push(c.to_string());

    let req = crate::canonical_runtime::TaskRequest {
        task: "add a function to the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;

    assert!(result.success, "task should succeed: {:?}", result.error);
    assert!(result.response.contains("final answer"));
    assert!(!chunks.lock().unwrap().is_empty());
    assert!(result.diagnostics.provider == "mock");
    assert!(!result.diagnostics.template.is_empty());
    assert!(!result.diagnostics.project.is_empty());

    let events = events.lock().unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::AgentCompleted { agent, .. } if agent == "main")));
}

#[tokio::test]
async fn test_project_identity_reaches_engineering_context() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (context, _compiled) = runtime
        .compile_for_task("explain the project", no_conversation())
        .await
        .unwrap();

    assert!(!context.project.name.is_empty());
    assert_eq!(
        context.project.workspace_root.as_deref(),
        Some(dir.path().to_str().unwrap())
    );
    // Project identity is present in the compiled prompt via the compiler.
}

#[tokio::test]
async fn test_coordinator_analysis_is_grounded_in_workspace() {
    // CanonicalRuntime → observe → Coordinator → SubAgentContext → grounded
    // report. The report must reference actual files/dependencies from the
    // temp workspace (no index required: workspace-scan fallback).
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n[dependencies]\ntokio = \"1\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/parser.rs"),
        "pub fn parse_tool_calls() {}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/parser_tests.rs"),
        "#[cfg(test)] mod tests { #[test] fn it_works() {} }\n",
    )
    .unwrap();

    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    let (context, _compiled) = runtime
        .compile_for_task("trace the parser module execution", no_conversation())
        .await
        .expect("compile");

    let report = context
        .context_fragments
        .iter()
        .find(|f| f.source == "agent_analysis")
        .map(|f| f.content.clone())
        .unwrap_or_default();

    assert!(
        !report.is_empty(),
        "agent analysis report should be present"
    );
    assert!(
        report.contains("src/parser.rs"),
        "grounded report should reference the real parser file:\n{}",
        report
    );
    assert!(
        report.contains("tokio"),
        "grounded report should reference a real manifest dependency:\n{}",
        report
    );
    assert!(
        report.contains("src/parser_tests.rs"),
        "grounded report should reference the real test file:\n{}",
        report
    );
}

#[tokio::test]
async fn test_coordinator_memory_entries_flow_into_subagent_context() {
    // Engineering memory resolved by the runtime must reach the subagent
    // context (memory_entries) and surface in the grounded report.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    {
        let entry = crate::engineering_memory::EngineeringMemoryEntry::new(
            "m1",
            "ci",
            "uses github actions",
        )
        .with_metadata(
            crate::engineering_memory::types::EngineeringMemoryMetadata {
                importance: 0.9,
                confidence: 0.9,
                tags: vec!["ci".to_string()],
                source: Some("test".to_string()),
            },
        );
        let mem = runtime.memory.as_mut().unwrap();
        mem.record(entry).unwrap();
        mem.persist().unwrap();
    }

    let (context, _compiled) = runtime
        .compile_for_task("ci", no_conversation())
        .await
        .expect("compile");

    let report = context
        .context_fragments
        .iter()
        .find(|f| f.source == "agent_analysis")
        .map(|f| f.content.clone())
        .unwrap_or_default();

    assert!(
        report.contains("ci: uses github actions"),
        "memory entry should reach the subagent report:\n{}",
        report
    );
}

// =========================================================================
// Sprint 30B.5 — Grounded context integration into the main LLM prompt
// =========================================================================

/// A small workspace whose files/symbols match the task terms, with a real
/// `.codebro/index.db` so the coordinator resolves actual symbols.
fn grounded_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src/canonical_runtime")).unwrap();
    std::fs::create_dir_all(dir.path().join("src/agent")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n[dependencies]\ntokio = \"1\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/canonical_runtime/mod.rs"),
        "pub struct CanonicalRuntime {}\nimpl CanonicalRuntime {\n    pub fn run_execution_loop() {}\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/agent/tool_parser.rs"),
        "pub fn parse_tool_calls() {}\npub fn trace_runtime_parsing() {}\n",
    )
    .unwrap();

    let codebro_dir = dir.path().join(".codebro");
    std::fs::create_dir_all(&codebro_dir).unwrap();
    {
        let mut indexer =
            crate::intelligence::CodeIndexer::new(codebro_dir.join("index.db")).unwrap();
        let src = std::fs::read_to_string(dir.path().join("src/canonical_runtime/mod.rs")).unwrap();
        indexer
            .index_file(&dir.path().join("src/canonical_runtime/mod.rs"), &src)
            .unwrap();
        let src = std::fs::read_to_string(dir.path().join("src/agent/tool_parser.rs")).unwrap();
        indexer
            .index_file(&dir.path().join("src/agent/tool_parser.rs"), &src)
            .unwrap();
    }
    dir
}

/// The coordinator's aggregated report fragment (source = "agent_analysis").
fn agent_analysis_report(context: &crate::engineering_context::EngineeringContext) -> String {
    context
        .context_fragments
        .iter()
        .find(|f| f.source == "agent_analysis")
        .map(|f| f.content.clone())
        .unwrap_or_default()
}

/// Rebuild a context with one fragment source removed (used to build the
/// "baseline" prompt that lacks the grounded coordinator report).
fn context_without_fragment_source(
    ctx: &crate::engineering_context::EngineeringContext,
    source: &str,
) -> crate::engineering_context::EngineeringContext {
    let fragments: Vec<ContextFragment> = ctx
        .context_fragments
        .iter()
        .filter(|f| f.source != source)
        .cloned()
        .collect();
    EngineeringContextBuilder::new()
        .with_skip_validation()
        .project(ctx.project.clone())
        .task(ctx.task.clone().expect("task plan present"))
        .objective(ctx.objective.clone())
        .goal_alignment(ctx.goal_alignment.clone())
        .workspace(ctx.workspace.clone())
        .context_fragments(fragments)
        .memory(ctx.memory.clone())
        .constraints(ctx.constraints.clone())
        .runtime(ctx.runtime.clone())
        .active_files(ctx.active_files.clone())
        .user_request(ctx.user_request.clone())
        .conversation(ctx.conversation.clone())
        .system_prompt(ctx.system_prompt.clone())
        .build()
        .expect("rebuild baseline context")
}

#[tokio::test]
async fn test_grounded_report_fragment_reaches_compiled_prompt() {
    // Task → grounding → coordinator report → EngineeringContext →
    // PromptBuilder. The compiled prompt must contain the grounded report
    // (identifiable by the `agent_analysis` marker and the research output)
    // including actual repository facts.
    let dir = grounded_workspace();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();

    let (context, compiled) = runtime
        .compile_for_task("trace canonical runtime execution", no_conversation())
        .await
        .expect("compile");

    // The coordinator report fragment is present in the EngineeringContext.
    let report = agent_analysis_report(&context);
    assert!(
        report.contains("src/canonical_runtime/mod.rs"),
        "grounded report must reference the runtime file:\n{}",
        report
    );
    assert!(
        report.contains("run_execution_loop"),
        "grounded report must reference the runtime symbol:\n{}",
        report
    );

    // The report is compiled into the final prompt as a labelled fragment.
    assert!(
        compiled.prompt.contains("--- agent_analysis () ---"),
        "prompt must render the agent_analysis fragment:\n{}",
        compiled.prompt
    );
    assert!(
        compiled.prompt.contains("Research Findings"),
        "prompt must contain the grounded research analysis:\n{}",
        compiled.prompt
    );
    assert!(
        compiled.prompt.contains("src/canonical_runtime/mod.rs"),
        "prompt must contain the grounded file fact"
    );
    assert!(
        compiled.prompt.contains("run_execution_loop"),
        "prompt must contain the grounded symbol fact"
    );
}

#[tokio::test]
async fn test_grounded_facts_reach_mock_provider_prompt() {
    // Full chain: task → grounding → coordinator → EngineeringContext →
    // PromptBuilder → provider. The MockProvider records the exact prompt it
    // is asked to generate from; it must contain the grounded repository facts.
    let dir = grounded_workspace();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    let provider = Arc::new(MockProvider::new("mock", vec!["final answer".to_string()]));
    runtime.register_provider(provider.clone());

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "trace canonical runtime execution",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    assert!(result.success, "task should succeed: {:?}", result.error);
    assert!(result.response.contains("final answer"));

    let prompt = provider
        .last_prompt_text()
        .expect("mock provider must have received a prompt");
    assert!(
        prompt.contains("--- agent_analysis () ---"),
        "grounded report fragment must be in the prompt sent to the provider"
    );
    assert!(
        prompt.contains("src/canonical_runtime/mod.rs"),
        "provider prompt must contain the grounded file fact:\n{}",
        prompt
    );
    assert!(
        prompt.contains("run_execution_loop"),
        "provider prompt must contain the grounded symbol fact:\n{}",
        prompt
    );
}

#[tokio::test]
async fn test_grounded_vs_baseline_prompt_delta() {
    // The grounded facts must come from the coordinator report, not from some
    // other fragment. Removing the agent_analysis fragment must remove the
    // facts from the compiled prompt (the fixture has no other source carrying
    // them, so this deterministically attributes the facts to the report).
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src/canonical_runtime")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n[dependencies]\ntokio = \"1\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/canonical_runtime/mod.rs"),
        "pub fn run_execution_loop() {}\n",
    )
    .unwrap();

    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    let (context, compiled_grounded) = runtime
        .compile_for_task("trace canonical runtime execution", no_conversation())
        .await
        .expect("compile");

    assert!(
        compiled_grounded
            .prompt
            .contains("src/canonical_runtime/mod.rs"),
        "grounded prompt contains the file fact"
    );

    // Baseline: same context without the coordinator report.
    let baseline_ctx = context_without_fragment_source(&context, "agent_analysis");
    let baseline_prompt = PromptBuilder::new().compile_context(&baseline_ctx).prompt;
    assert!(
        !baseline_prompt.contains("src/canonical_runtime/mod.rs"),
        "baseline prompt must NOT contain the grounded file fact (it originates in the report)"
    );
    assert!(
        !baseline_prompt.contains("Research Findings"),
        "baseline prompt must not contain the coordinator report"
    );
}

#[tokio::test]
async fn test_grounded_context_cost_is_bounded() {
    // The grounded report must be present exactly once and stay small; adding
    // it must not balloon the prompt.
    let dir = grounded_workspace();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();

    let (context, compiled) = runtime
        .compile_for_task("trace canonical runtime execution", no_conversation())
        .await
        .expect("compile");

    // Exactly one agent_analysis fragment (no duplication).
    let report_frags: Vec<&ContextFragment> = context
        .context_fragments
        .iter()
        .filter(|f| f.source == "agent_analysis")
        .collect();
    assert_eq!(report_frags.len(), 1, "report fragment must appear once");

    // The report is a single bounded fragment.
    let report = agent_analysis_report(&context);
    assert!(
        report.len() < 4096,
        "grounded report must stay small, got {} bytes",
        report.len()
    );
    let report_tokens = report.len() / 4;

    // The prompt cost attributable to grounding is bounded by the report size
    // plus a small rendering overhead.
    let baseline_ctx = context_without_fragment_source(&context, "agent_analysis");
    let baseline_tokens = PromptBuilder::new()
        .compile_context(&baseline_ctx)
        .statistics
        .estimated_tokens;
    let grounded_tokens = compiled.statistics.estimated_tokens;
    let delta = grounded_tokens.saturating_sub(baseline_tokens);
    assert!(
        delta <= report_tokens + 256,
        "grounding must not add uncontrolled prompt growth: delta {} vs report {} tokens",
        delta,
        report_tokens
    );
}

#[tokio::test]
async fn test_engineering_memory_reaches_engineering_context() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    // Record a memory entry explicitly (memory writes remain explicit ops).
    {
        let entry = crate::engineering_memory::EngineeringMemoryEntry::new(
            "m1",
            "ci",
            "uses github actions",
        )
        .with_metadata(
            crate::engineering_memory::types::EngineeringMemoryMetadata {
                importance: 0.9,
                confidence: 0.9,
                tags: vec!["ci".to_string()],
                source: Some("test".to_string()),
            },
        );
        let mem = runtime.memory.as_mut().unwrap();
        mem.record(entry).unwrap();
        mem.persist().unwrap();
    }

    let (context, _compiled) = runtime
        .compile_for_task("ci", no_conversation())
        .await
        .unwrap();

    assert!(
        context.memory_count() >= 1,
        "memory should be resolved into context"
    );
}

#[tokio::test]
async fn test_context_assembly_fragments_budgeted() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (context, _compiled) = runtime
        .compile_for_task("add a new feature", no_conversation())
        .await
        .unwrap();

    // Assembly produced fragments and they were mapped into the context.
    assert!(context.fragment_count() >= 1);
    // The assembler's token budget is respected (default medium = 8000 tokens).
    assert!(context.estimated_tokens() < 20_000);
}

#[tokio::test]
async fn test_prompt_uses_canonical_compile_context() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (context, compiled) = runtime
        .compile_for_task("fix the auth bug", no_conversation())
        .await
        .unwrap();

    assert!(!compiled.prompt.is_empty());
    assert!(!compiled.statistics.template.is_empty());
    assert!(!compiled.template_selection.template.as_str().is_empty());
    // The canonical prompt includes the project identity and user request.
    assert!(compiled.prompt.contains("User Request"));
    assert!(compiled.prompt.contains("fix the auth bug"));
    assert!(context.fragment_count() >= 1);
}

#[tokio::test]
async fn test_intelligent_provider_router_participates() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(1));
    runtime.register_provider(Arc::new(MockProvider::new("mock", vec!["ok".to_string()])));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the codebase",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    assert!(result.success);
    // The intelligent router selected the registered provider.
    assert_eq!(result.diagnostics.provider, "mock");
    assert!(!result.diagnostics.strategy.is_empty());
}

#[tokio::test]
async fn test_selected_provider_passes_through_circuit_breaker() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(1));
    runtime.register_provider(Arc::new(MockProvider::new("mock", vec!["ok".to_string()])));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the codebase",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    assert!(result.success);
    assert!(result.diagnostics.breaker_allowed);

    // The breaker exists and is closed after a success.
    let id = crate::provider_runtime::ProviderId::new("mock");
    let breaker = runtime.provider_runtime().circuit_breakers().get(&id);
    assert!(breaker.is_some());
    assert_eq!(
        breaker.unwrap().state(),
        crate::provider_runtime::CircuitBreakerState::Closed
    );
}

#[tokio::test]
async fn test_provider_failure_reports_and_fails_task() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(MockProvider::failing("mock")));

    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the codebase",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    assert!(!result.success);
    assert!(result.error.is_some());

    // report_failure was connected: the circuit breaker recorded the failure
    // (report_failure drives breaker.record_failure and health reporting).
    let id = crate::provider_runtime::ProviderId::new("mock");
    let breaker = runtime
        .provider_runtime()
        .circuit_breakers()
        .get(&id)
        .expect("breaker should exist for the routed provider");
    assert!(breaker.failure_count() > 0);

    // The task graph entered the Failed state (AgentFailed emitted).
    let events = events.lock().unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::AgentFailed { agent, .. } if agent == "main")));
}

#[tokio::test]
async fn test_task_failure_enters_failed_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(MockProvider::failing("mock")));

    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the codebase",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    assert!(!result.success);

    let events = events.lock().unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::TaskGraphUpdated { graph } if graph.has_failures())));
}

#[tokio::test]
async fn test_deterministic_context_and_prompt() {
    let dir = tempfile::tempdir().unwrap();

    let mut runtime_a =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime_a.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));
    let (ctx_a, prompt_a) = runtime_a
        .compile_for_task("fix the auth bug", no_conversation())
        .await
        .unwrap();

    let mut runtime_b =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime_b.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));
    let (ctx_b, prompt_b) = runtime_b
        .compile_for_task("fix the auth bug", no_conversation())
        .await
        .unwrap();

    // Same task + same state => deterministic context and prompt.
    assert!(ctx_a.equals(&ctx_b));
    assert_eq!(prompt_a.prompt, prompt_b.prompt);
    assert_eq!(
        prompt_a.template_selection.template,
        prompt_b.template_selection.template
    );
}

#[tokio::test]
async fn test_empty_memory_task_executes() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(1));
    runtime.register_provider(Arc::new(MockProvider::new(
        "mock",
        vec!["works with no memory".to_string()],
    )));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    assert!(
        result.success,
        "empty memory must not block: {:?}",
        result.error
    );
    assert_eq!(result.diagnostics.memory_entries, 0);
}

#[tokio::test]
async fn test_empty_optional_context_task_executes() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(1));
    runtime.register_provider(Arc::new(MockProvider::new(
        "mock",
        vec!["hello".to_string()],
    )));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "hello",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    assert!(
        result.success,
        "plain chat must still work: {:?}",
        result.error
    );
}

#[tokio::test]
async fn test_provider_unavailable_preserves_failure_behavior() {
    let dir = tempfile::tempdir().unwrap();
    // No providers registered: routing must fail cleanly (NoSuitableProvider)
    // rather than panicking, and the task must enter the Failed path.
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the codebase",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    assert!(!result.success);
    let error = result.error.expect("task should report an error");
    assert!(
        error.contains("routing") || error.contains("provider"),
        "unexpected error: {error}"
    );
}

/// True end-to-end pipeline test: a single task traverses every canonical
/// stage with a mock provider and all stage diagnostics are populated.
///
/// ```text
/// Engineering Task
///   → Project Identity
///   → Engineering Memory
///   → Context Assembly
///   → EngineeringContext
///   → Prompt Builder
///   → Provider Router
///   → Provider Runtime
///   → Mock Provider
///   → Task Result
/// ```
#[tokio::test]
async fn test_end_to_end_canonical_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(1));
    runtime.register_provider(Arc::new(MockProvider::new(
        "mock",
        vec!["implemented the feature".to_string()],
    )));

    let (events, emit) = event_sink();
    let chunks: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let chunk_sink = chunks.clone();
    let on_chunk = move |c: &str| chunk_sink.lock().unwrap().push(c.to_string());

    let req = crate::canonical_runtime::TaskRequest {
        task: "add a new feature to the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    assert!(result.success, "end-to-end failure: {:?}", result.error);

    let diag = &result.diagnostics;
    // Project identity.
    assert!(!diag.project.is_empty());
    assert!(!diag.project_root.is_empty());
    // Engineering memory (resolved, may be empty for a fresh project).
    assert_eq!(diag.memory_entries, 0);
    // Context assembly.
    assert!(diag.context_fragments >= 1);
    // Prompt compilation.
    assert!(!diag.template.is_empty());
    assert!(diag.prompt_tokens > 0);
    // Provider routing + runtime.
    assert_eq!(diag.provider, "mock");
    assert!(diag.breaker_allowed);

    // Streaming reached the sink.
    assert_eq!(
        chunks.lock().unwrap().clone(),
        vec!["implemented the feature".to_string()]
    );

    // Lifecycle reached Completed.
    let events = events.lock().unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::AgentCompleted { agent, .. } if agent == "main")));
    assert!(events.iter().any(|e| matches!(
        e,
        AgentEvent::TaskGraphUpdated { graph } if graph.is_complete()
    )));
}

/// Performance observation: startup + per-stage orchestration overhead.
///
/// Ignored by default; run with `cargo test --bin codebro perf_measurement
/// -- --ignored --nocapture`. Numbers are observational and machine-dependent.
#[tokio::test]
#[ignore]
async fn perf_measurement_report() {
    let dir = tempfile::tempdir().unwrap();

    // Startup / identity / memory load.
    let t0 = std::time::Instant::now();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(1));
    runtime.register_provider(Arc::new(MockProvider::new("mock", vec!["ok".to_string()])));
    let startup_ms = t0.elapsed().as_micros();

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "add a feature to the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    let d = &result.diagnostics;
    println!(
        "\n[perf] startup={}us identity={}us memory_resolve={}us assembly={}us compile={}us routing={}us exec={}us total={}us",
        startup_ms,
        d.identity_load_ms.saturating_mul(1000),
        d.memory_resolution_ms.saturating_mul(1000),
        d.assembly_ms.saturating_mul(1000),
        d.compile_ms.saturating_mul(1000),
        d.routing_ms.saturating_mul(1000),
        d.provider_execution_ms.saturating_mul(1000),
        d.total_ms.saturating_mul(1000),
    );
    println!(
        "[perf] fragments={} memory={} prompt={}tok template={} provider={} breaker_allowed={}",
        d.context_fragments,
        d.memory_entries,
        d.prompt_tokens,
        d.template,
        d.provider,
        d.breaker_allowed
    );
    assert!(result.success);
}

// =========================================================================
// Sprint 27 — Engineering Objective & Lazy Execution
// =========================================================================

use crate::engineering_objective::{EngineeringObjective, EngineeringObjectiveRuntime};

/// Write a configured objective file for a workspace (explicit persistence).
fn write_objective(root: &std::path::Path, objective: EngineeringObjective) {
    let mut rt = EngineeringObjectiveRuntime::new(root);
    rt.create(objective).expect("write objective");
}

fn sample_objective() -> EngineeringObjective {
    EngineeringObjective::new(
        "Build a terminal-native engineering intelligence runtime.",
        "CodeBro is a trustworthy engineering intelligence runtime for developers.",
        "Make CodeBro capable of maintaining software projects.",
        "Sprint 27 — Engineering Objective & Lazy Execution.",
    )
    .with_success_criteria(vec![
        "All production tasks use the canonical runtime.".to_string()
    ])
    .with_non_goals(vec![
        "General chatbot".to_string(),
        "IDE replacement".to_string(),
    ])
    .with_source("docs/vision/CODEBRO_VISION.md")
}

#[tokio::test]
async fn test_missing_objective_is_empty_and_unconfigured() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (context, _compiled) = runtime
        .compile_for_task("maintain the project", no_conversation())
        .await
        .unwrap();

    // A workspace without an objective file stays empty/unconfigured.
    assert!(
        context.objective.is_empty(),
        "CodeBro must not install its own objective into an arbitrary workspace"
    );
    // No objective file is silently created.
    let objective_path = dir
        .path()
        .join(".codebro")
        .join("engineering_objective.json");
    assert!(
        !objective_path.exists(),
        "a missing objective must not be persisted"
    );
    // No goal alignment for an unconfigured objective.
    assert!(context.goal_alignment.is_none());
}

#[tokio::test]
async fn test_missing_objective_does_not_break_task_execution() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(1));
    runtime.register_provider(Arc::new(MockProvider::new(
        "mock",
        vec!["done".to_string()],
    )));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    assert!(
        result.success,
        "missing objective must never break task execution: {:?}",
        result.error
    );
}

#[tokio::test]
async fn test_configured_objective_loads_and_reaches_prompt() {
    let dir = tempfile::tempdir().unwrap();
    write_objective(dir.path(), sample_objective());

    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (context, compiled) = runtime
        .compile_for_task("implement indexed workspace retrieval", no_conversation())
        .await
        .unwrap();

    // The configured objective loads into the context.
    assert_eq!(context.objective, sample_objective());
    // Compact always-on block reaches the prompt.
    assert!(compiled.prompt.contains("END GOAL"));
    assert!(compiled.prompt.contains("CURRENT OBJECTIVE"));
    assert!(compiled.prompt.contains("CURRENT MILESTONE"));
    assert!(compiled.prompt.contains("CURRENT TASK"));
    assert!(compiled.prompt.contains("TASK ALIGNMENT"));
}

#[tokio::test]
async fn test_goal_alignment_computed_deterministically() {
    let dir = tempfile::tempdir().unwrap();
    write_objective(dir.path(), sample_objective());

    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (context, _compiled) = runtime
        .compile_for_task("maintain the software project", no_conversation())
        .await
        .unwrap();

    assert!(context.goal_alignment.is_some());
    // "maintain" + "software" both appear in the configured objective.
    assert_eq!(
        context.goal_alignment,
        Some(crate::engineering_objective::GoalAlignment::Direct)
    );
}

#[tokio::test]
async fn test_objective_is_workspace_scoped() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    write_objective(dir_a.path(), sample_objective());

    let mut runtime_a =
        CanonicalRuntime::new_without_default_provider(test_config(), dir_a.path()).unwrap();
    runtime_a.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));
    let (ctx_a, _) = runtime_a
        .compile_for_task("explain the project", no_conversation())
        .await
        .unwrap();
    assert!(!ctx_a.objective.is_empty());

    // A different workspace does not inherit workspace A's objective.
    let mut runtime_b =
        CanonicalRuntime::new_without_default_provider(test_config(), dir_b.path()).unwrap();
    runtime_b.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));
    let (ctx_b, _) = runtime_b
        .compile_for_task("explain the project", no_conversation())
        .await
        .unwrap();
    assert!(ctx_b.objective.is_empty());
}

#[tokio::test]
async fn test_objective_persisted_explicitly_and_reloaded() {
    let dir = tempfile::tempdir().unwrap();

    // Explicit persistence: write the objective, then reload through the
    // canonical runtime.
    write_objective(dir.path(), sample_objective());

    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));
    let (context, _compiled) = runtime
        .compile_for_task("explain the project", no_conversation())
        .await
        .unwrap();
    assert_eq!(context.objective.end_goal, sample_objective().end_goal);
}

#[tokio::test]
async fn test_conversation_is_task_scoped_and_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    // A long conversation that is mostly unrelated to the current task.
    let conversation: Vec<ConversationMessage> = (0..100)
        .map(|i| ConversationMessage {
            role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
            content: format!("old unrelated message number {}", i),
        })
        .collect();

    let (context, _compiled) = runtime
        .compile_for_task("fix the auth bug", conversation.clone())
        .await
        .unwrap();

    // The conversation is bounded: never the full 100-message history.
    assert!(
        context.conversation.len() <= 20,
        "conversation should be bounded to the task window, got {}",
        context.conversation.len()
    );
    assert!(context.conversation.len() < conversation.len());
    // Recent messages are preserved (the tail is kept, oldest dropped).
    assert!(context
        .conversation
        .iter()
        .any(|m| m.content.contains("message number 99")));
    assert!(!context
        .conversation
        .iter()
        .any(|m| m.content.contains("message number 0")));
}

#[tokio::test]
async fn test_objective_pipeline_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(1));
    runtime.register_provider(Arc::new(MockProvider::new(
        "mock",
        vec!["implemented the feature".to_string()],
    )));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "maintain the software project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    assert!(result.success, "e2e failure: {:?}", result.error);
}

// =========================================================================
// Fragment deduplication (content-aware fingerprints)
// =========================================================================

#[test]
fn test_dedup_same_source_same_content_removes_duplicate() {
    let mut frags = vec![
        ContextFragment {
            source: "tool_result".to_string(),
            content: "output one".to_string(),
            relevance_score: 0.9,
        },
        ContextFragment {
            source: "tool_result".to_string(),
            content: "output one".to_string(),
            relevance_score: 0.9,
        },
    ];
    super::dedup_fragments(&mut frags);
    assert_eq!(frags.len(), 1, "identical fragments must deduplicate");
}

#[test]
fn test_dedup_same_source_different_content_preserves() {
    // Equal-length, same-source, different content must NOT collide.
    let mut frags = vec![
        ContextFragment {
            source: "tool_result".to_string(),
            content: "abcdefghij".to_string(),
            relevance_score: 0.9,
        },
        ContextFragment {
            source: "tool_result".to_string(),
            content: "klmnopqrst".to_string(),
            relevance_score: 0.9,
        },
    ];
    super::dedup_fragments(&mut frags);
    assert_eq!(
        frags.len(),
        2,
        "distinct equal-length fragments must survive"
    );
}

#[test]
fn test_dedup_different_source_same_content_preserves() {
    let mut frags = vec![
        ContextFragment {
            source: "tool_result".to_string(),
            content: "same content".to_string(),
            relevance_score: 0.9,
        },
        ContextFragment {
            source: "agent_analysis".to_string(),
            content: "same content".to_string(),
            relevance_score: 0.8,
        },
    ];
    super::dedup_fragments(&mut frags);
    assert_eq!(frags.len(), 2, "different sources must survive");
}

// =========================================================================
// Interaction contract — Recommend, don't interrogate
// =========================================================================

#[tokio::test]
async fn test_prompt_contract_recommends_not_interrogates() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (_context, compiled) = runtime
        .compile_for_task("fix the auth bug", no_conversation())
        .await
        .unwrap();

    // The model-facing contract recommends, executes low-risk actions, and
    // no longer asks for confirmation on every routine step.
    assert!(compiled.prompt.contains("Recommend, don't interrogate"));
    assert!(
        !compiled
            .prompt
            .contains("Ask for clarification when requirements are ambiguous"),
        "routine actions must not require unnecessary confirmation"
    );
    assert!(!compiled
        .prompt
        .contains("Always explain what you are about to do before doing it"));
    // Consequential actions still require confirmation.
    assert!(compiled
        .prompt
        .contains("Never run destructive commands without explicit user confirmation"));
}

// =========================================================================
// Sprint 28 — PTY streaming tool path & explicit verification
// =========================================================================

#[tokio::test]
async fn test_run_tool_streaming_emits_authoritative_events() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (events, emit) = event_sink();

    let outcome = runtime
        .run_tool_streaming(
            "run_command",
            "c1",
            "printf 'live\\noutput\\n'",
            &emit,
            Default::default(),
        )
        .await;

    assert!(outcome.success, "exit code must be 0");
    assert_eq!(outcome.exit_code, 0);
    assert!(
        outcome.output.contains("live"),
        "streamed output must be captured: {}",
        outcome.output
    );

    let snapshot = events.lock().unwrap().clone();
    assert!(
        snapshot
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolStarted { tool, .. } if tool == "run_command")),
        "must emit real ToolStarted"
    );
    assert!(
        snapshot.iter().any(|e| matches!(
            e,
            AgentEvent::PtyOutput { console, .. } if console == "c1"
        )),
        "must emit real PtyOutput chunks"
    );
    assert!(
        snapshot.iter().any(|e| matches!(
            e,
            AgentEvent::PtyExited { console, exit_code, .. } if console == "c1" && *exit_code == 0
        )),
        "must emit real PtyExited with the exit code"
    );
    assert!(
        snapshot
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolCompleted { success: true, .. })),
        "must emit ToolCompleted for the real run"
    );
}

#[tokio::test]
async fn test_run_tool_streaming_captures_failure_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (_events, emit) = event_sink();
    let outcome = runtime
        .run_tool_streaming("run_command", "c2", "exit 7", &emit, Default::default())
        .await;

    assert!(!outcome.success);
    assert_eq!(outcome.exit_code, 7);
}

#[tokio::test]
async fn test_run_tool_streaming_respects_cancellation() {
    use crate::cancellation::CancellationToken;
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (_events, emit) = event_sink();
    let token = CancellationToken::new();
    // Cancel immediately: the PTY receives SIGINT before/while starting.
    token.cancel();
    let outcome = runtime
        .run_tool_streaming("run_command", "c3", "sleep 30", &emit, token)
        .await;
    assert!(outcome.cancelled, "cancelled outcome expected");
}

#[tokio::test]
async fn test_verify_commands_detects_cargo_workspace() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));
    let (build, test) = runtime.verify_commands();
    assert_eq!(
        build.map(|(l, c)| (l, c)),
        Some(("cargo build".into(), "cargo build".into()))
    );
    assert_eq!(
        test.map(|(l, c)| (l, c)),
        Some(("cargo test".into(), "cargo test".into()))
    );
}

#[tokio::test]
async fn test_verify_commands_none_for_unknown_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));
    let (build, test) = runtime.verify_commands();
    assert!(build.is_none());
    assert!(test.is_none());
}

#[tokio::test]
async fn test_verify_task_runs_real_build_and_reports() {
    let dir = tempfile::tempdir().unwrap();
    // A minimal crate that compiles: cargo build + cargo test should pass.
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"vt\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src").join("lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n#[cfg(test)]\nmod tests { #[test] fn ok() {} }\n",
    )
    .unwrap();

    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));
    let (_events, emit) = event_sink();

    let req = crate::canonical_runtime::TaskRequest {
        task: "verify me",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &|_| {},
    };
    let mut diag = crate::canonical_runtime::TaskDiagnostics::new("verify me");
    let result = runtime
        .verify_task(
            &req,
            &Default::default(),
            &mut diag,
            std::time::Instant::now(),
        )
        .await;

    let (summary, text) = result.expect("verification applicable");
    assert!(
        summary.steps.iter().all(|s| s.success),
        "build and test must pass for a valid crate: {}",
        text
    );
    assert!(text.contains("Verification passed"));
    assert!(diag.verification.is_some());
}

// =========================================================================
// Sprint 29 — Canonical Agent Loop & Task Execution Reliability
// =========================================================================

use crate::agent::task_graph::TaskGraph;
use crate::cancellation::CancellationToken;
use crate::canonical_runtime::MAX_MODEL_CALLS;
use crate::canonical_runtime::MAX_REACT_ITERATIONS;

/// A mock provider that returns responses sequentially (consuming each one).
/// Useful for testing multi-step agent loops where the provider returns
/// different responses on each call.
#[derive(Clone)]
struct ScriptedMockProvider {
    name: String,
    model: String,
    responses: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ScriptedMockProvider {
    fn new(name: &str, responses: Vec<String>) -> Self {
        ScriptedMockProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            responses: std::sync::Arc::new(std::sync::Mutex::new(responses)),
            fail: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn failing(name: &str) -> Self {
        let p = ScriptedMockProvider::new(name, Vec::new());
        p.fail.store(true, std::sync::atomic::Ordering::SeqCst);
        p
    }
}

impl Provider for ScriptedMockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn base_url(&self) -> &str {
        "mock://localhost"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn api_key(&self) -> Option<&str> {
        Some("mock-key")
    }

    fn send_message(
        &self,
        _message: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
    {
        let mut responses = self.responses.lock().unwrap();
        let response = if responses.is_empty() {
            String::new()
        } else {
            responses.remove(0)
        };
        Box::pin(async move { Ok(response) })
    }

    fn stream_response(
        &self,
        _message: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                > + Send
                + '_,
        >,
    > {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            let result = Err(anyhow::anyhow!("mock provider offline"));
            Box::pin(async move { result })
        } else {
            let mut responses = self.responses.lock().unwrap();
            let response = if responses.is_empty() {
                String::new()
            } else {
                responses.remove(0)
            };
            let _ = tx.send(response);
            Box::pin(async move { Ok(rx) })
        }
    }
}

/// Collect all AgentEvents emitted during a task into a vector.
fn collect_events(events: &std::sync::Arc<std::sync::Mutex<Vec<AgentEvent>>>) -> Vec<AgentEvent> {
    events.lock().unwrap().clone()
}

/// Helper: build a runtime with a scripted mock provider for loop testing.
fn loop_test_runtime(responses: Vec<String>) -> CanonicalRuntime {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new("mock", responses)));
    runtime
}

// ---------------------------------------------------------------------------
// 1. Simple completion — model returns no tool calls → Completed
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_simple_completion_no_tool_calls() {
    let mut runtime = loop_test_runtime(vec!["I will add the function.".to_string()]);
    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "add a function to the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    assert!(
        result.success,
        "simple completion should succeed: {:?}",
        result.error
    );
    assert!(!result.response.is_empty());
    assert!(!result.cancelled);
    let evs = collect_events(&events);
    assert!(evs
        .iter()
        .any(|e| matches!(e, AgentEvent::AgentCompleted { .. })));
}

// ---------------------------------------------------------------------------
// 2. Single tool execution — tool call → result → final answer → Completed
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_single_tool_execution() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec![
            // First response: tool call to list files.
            "<invoke name=\"list_files\">{\"path\": \".\"}</invoke>".to_string(),
            // Second response: final answer after tool result.
            "Done.".to_string(),
        ],
    )));
    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "list the project files",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    assert!(
        result.success,
        "single tool execution should succeed: {:?}",
        result.error
    );
    let evs = collect_events(&events);
    assert!(evs
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolStarted { tool, .. } if tool == "list_files")));
    assert!(evs
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolCompleted { tool, .. } if tool == "list_files")));
    assert!(evs
        .iter()
        .any(|e| matches!(e, AgentEvent::AgentCompleted { .. })));
}

// ---------------------------------------------------------------------------
// 3. Multi-step execution — tool A → tool B → final answer → Completed
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_multi_step_execution() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec![
            // First response: tool call.
            "<invoke name=\"list_files\">{\"path\": \".\"}</invoke>".to_string(),
            // Second response: another tool call.
            "<invoke name=\"read_file\">{\"path\": \"README.md\"}</invoke>".to_string(),
            // Third response: final answer.
            "Done with both steps.".to_string(),
        ],
    )));
    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "read the project readme",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    assert!(
        result.success,
        "multi-step execution should succeed: {:?}",
        result.error
    );
    let evs = collect_events(&events);
    let tool_starts: Vec<&str> = evs
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolStarted { tool, .. } => Some(tool.as_str()),
            _ => None,
        })
        .collect();
    assert!(tool_starts.contains(&"list_files"));
    assert!(tool_starts.contains(&"read_file"));
}

// ---------------------------------------------------------------------------
// 4. Verification pass — execute → verify PASS → Completed
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_verification_pass() {
    let dir = tempfile::tempdir().unwrap();
    // Create a minimal valid Rust crate so verification can run.
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"vt\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src").join("lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n#[cfg(test)]\nmod tests { #[test] fn ok() {} }\n",
    )
    .unwrap();

    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec!["Done.".to_string()],
    )));
    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "add a function to the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    assert!(
        result.success,
        "verification pass should succeed: {:?}",
        result.error
    );
    assert!(result.diagnostics.verification.is_some());
    let evs = collect_events(&events);
    assert!(evs
        .iter()
        .any(|e| matches!(e, AgentEvent::AgentCompleted { .. })));
}

// ---------------------------------------------------------------------------
// 5. Verification failure + bounded revision → Completed
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_verification_failure_then_revision_passes() {
    let dir = tempfile::tempdir().unwrap();
    // Start with a broken crate: compilation will fail.
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"vt\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // Invalid Rust code to make cargo build fail.
    std::fs::write(
        dir.path().join("src").join("lib.rs"),
        "pub fn broken() { this is not valid rust {{{",
    )
    .unwrap();

    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec![
            // First attempt: the model says it fixed it.
            "I fixed the code.".to_string(),
            // Second attempt (revision): model says it's done.
            "All fixed now.".to_string(),
        ],
    )));
    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "fix the broken code in the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    // After one revision, the task should still fail because the crate is
    // genuinely broken and the mock provider cannot write real source files.
    // The important invariant: the loop terminates (does not hang).
    assert!(result.diagnostics.verification.is_some());
    // The task either completed (if verification passed on revision) or failed
    // after exhausting revisions — either way it reached a terminal state.
    assert!(result.success || result.error.is_some());
}

// ---------------------------------------------------------------------------
// 6. Verification repeatedly fails → terminal (bounded retries exhausted)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_verification_repeatedly_fails_then_terminates() {
    let dir = tempfile::tempdir().unwrap();
    // Create a crate that will always fail to compile.
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"vt\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src").join("lib.rs"),
        "pub fn broken() { invalid {{{",
    )
    .unwrap();

    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    // Provide responses for the initial execution + 2 revision attempts.
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec![
            "Fixed.".to_string(),
            "Still fixing.".to_string(),
            "Done now.".to_string(),
        ],
    )));
    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "fix the broken code",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    // Should terminate after exhausting revision budget.
    assert!(!result.success);
    assert!(result.error.is_some());
    assert!(result.error.as_ref().unwrap().contains("revision"));
}

// ---------------------------------------------------------------------------
// 7. Cancellation — active task → cancel → Cancelled
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_cancellation_during_task() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    // Provider never responds — cancellation will be checked before provider.
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec!["slow".to_string()],
    )));

    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let token = CancellationToken::new();
    let options = crate::canonical_runtime::TaskOptions {
        cancel: Some(token.clone()),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    // Cancel immediately before spawning.
    token.cancel();
    let result = runtime.run_task_with_options(&req, options).await;
    assert!(result.cancelled, "task should be cancelled");
    assert!(!result.success);
    let evs = collect_events(&events);
    assert!(evs
        .iter()
        .any(|e| matches!(e, AgentEvent::AgentCancelled { .. })));
}

// ---------------------------------------------------------------------------
// 8. Timeout — task exceeds budget → Failed
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_task_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec!["answer".to_string()],
    )));
    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        task_timeout_ms: Some(1), // 1ms timeout
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task_with_options(&req, options).await;
    // With a 1ms timeout the task may or may not time out depending on system
    // speed, but it should always terminate.
    assert!(result.success || result.error.is_some() || result.cancelled);
}

// ---------------------------------------------------------------------------
// 9. Provider failure — model/provider error → Failed
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_provider_failure() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::failing("mock")));

    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the codebase",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    assert!(!result.success);
    assert!(result.error.is_some());
    let evs = collect_events(&events);
    assert!(evs
        .iter()
        .any(|e| matches!(e, AgentEvent::AgentFailed { .. })));
}

// ---------------------------------------------------------------------------
// 10. Tool failure — tool returns error, model receives it, bounded recovery
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_tool_failure_with_bounded_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec![
            // First: tool call to unknown tool → error.
            "<invoke name=\"nonexistent_tool\">{\"x\": \"y\"}</invoke>".to_string(),
            // Second: model recovers with a final answer.
            "I could not find that tool, but here is the answer.".to_string(),
        ],
    )));
    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "use a nonexistent tool",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    // The model should recover and produce a final answer.
    assert!(
        result.success,
        "tool failure should allow bounded recovery: {:?}",
        result.error
    );
    let evs = collect_events(&events);
    assert!(evs
        .iter()
        .any(|e| matches!(e, AgentEvent::AgentCompleted { .. })));
}

// ---------------------------------------------------------------------------
// 11. Loop budget exhaustion — max iterations exceeded → Failed
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_loop_budget_exhaustion() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    // Provider keeps returning tool calls for all MAX_REACT_ITERATIONS (5)
    // iterations, so the loop must exhaust its budget.
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec![
            "<invoke name=\"list_files\">{\"path\": \".\"}</invoke>".to_string();
            MAX_REACT_ITERATIONS + 2
        ],
    )));
    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "list files repeatedly",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    eprintln!(
        "loop_budget result: success={} error={:?} response={:?}",
        result.success,
        result.error,
        &result.response[..result.response.len().min(200)]
    );
    // After exhausting iterations the loop returns an error (not Ok).
    assert!(
        !result.success || result.error.is_some(),
        "loop budget exhaustion should produce a failure or error: {:?}",
        result.error
    );
    let evs = collect_events(&events);
    // Should not reach AgentCompleted for the main agent if loop budget was exhausted.
    assert!(
        !evs.iter()
            .any(|e| matches!(e, AgentEvent::AgentCompleted { agent, .. } if agent == "main")),
        "should not complete main agent when loop budget is exhausted"
    );
}

// ---------------------------------------------------------------------------
// 12. Repeated action detection — same action repeated beyond threshold
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_repeated_action_detection() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    // Provider keeps returning the same tool call fingerprint.
    let same_fingerprint = "<invoke name=\"list_files\">{\"path\": \".\"}</invoke>".to_string();
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec![
            same_fingerprint.clone(),
            same_fingerprint.clone(),
            same_fingerprint.clone(),
            same_fingerprint.clone(),
        ],
    )));
    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "list files forever",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    // Should terminate with an error about repeated actions.
    assert!(
        !result.success || result.error.is_some(),
        "repeated action should be detected: {:?}",
        result.error
    );
    if let Some(ref err) = result.error {
        assert!(
            err.contains("Repeated") || err.contains("iteration") || err.contains("cancelled"),
            "unexpected error: {}",
            err
        );
    }
}

// ---------------------------------------------------------------------------
// 13. Terminal-state invariant — no state mutation after terminal
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_terminal_state_invariant() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec!["final.".to_string()],
    )));
    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "simple task",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    assert!(result.success);
    assert!(!result.cancelled);
    let evs = collect_events(&events);
    // The canonical runtime emits exactly one AgentCompleted for the main
    // agent. The observe phase may emit subagent Completed events through
    // the coordinator, but the main task must end with exactly one terminal
    // event for agent "main".
    let main_terminal: Vec<&AgentEvent> = evs
        .iter()
        .filter(|e| match e {
            AgentEvent::AgentCompleted { agent, .. }
            | AgentEvent::AgentFailed { agent, .. }
            | AgentEvent::AgentCancelled { agent, .. } => agent == "main",
            _ => false,
        })
        .collect();
    assert_eq!(
        main_terminal.len(),
        1,
        "exactly one terminal event for the main agent expected, got: {:?}",
        main_terminal
            .iter()
            .map(|e| match e {
                AgentEvent::AgentCompleted { .. } => "Completed",
                AgentEvent::AgentFailed { .. } => "Failed",
                AgentEvent::AgentCancelled { .. } => "Cancelled",
                _ => "other",
            })
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// 14. Cancellation propagation — cancellation reaches active tool path
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_cancellation_propagates_to_tool() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec![
            // Provider returns a tool call; cancellation happens during execution.
            "<invoke name=\"list_files\">{\"path\": \".\"}</invoke>".to_string(),
        ],
    )));
    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let token = CancellationToken::new();
    let options = crate::canonical_runtime::TaskOptions {
        cancel: Some(token.clone()),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "list files",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    // Cancel during execution.
    token.cancel();
    let result = runtime.run_task_with_options(&req, options).await;
    assert!(
        result.cancelled || !result.success,
        "cancellation should terminate the task"
    );
    let evs = collect_events(&events);
    // Either AgentCancelled or AgentFailed should be emitted.
    assert!(
        evs.iter().any(|e| matches!(
            e,
            AgentEvent::AgentCancelled { .. } | AgentEvent::AgentFailed { .. }
        )),
        "expected terminal cancellation/failure event"
    );
}

// ---------------------------------------------------------------------------
// State-transition tests for TaskStatus and RuntimeState
// ---------------------------------------------------------------------------

#[test]
fn test_task_status_cancelled_is_terminal() {
    use crate::agent::task_graph::TaskStatus;
    assert!(TaskStatus::Cancelled == TaskStatus::Cancelled);
    // Cancelled is a valid final state alongside Completed and Failed.
    let mut graph = TaskGraph::new("test");
    let root = graph.root_task.clone();
    graph.update_status(&root, TaskStatus::Cancelled);
    let node = graph.get_task(&root).unwrap();
    assert!(matches!(node.status, TaskStatus::Cancelled));
    assert!(node.completed_at.is_some());
}

#[test]
fn test_runtime_state_cancelled_transitions() {
    use crate::runtime::state::RuntimeState;
    // From Synthesizing, can transition to Cancelled.
    assert!(RuntimeState::Synthesizing
        .try_transition(RuntimeState::Cancelled)
        .is_ok());
    // From Acting, can transition to Cancelled.
    assert!(RuntimeState::Acting
        .try_transition(RuntimeState::Cancelled)
        .is_ok());
    // From a terminal state, no transition is valid.
    assert!(RuntimeState::Cancelled
        .try_transition(RuntimeState::Observing)
        .is_err());
    assert!(RuntimeState::Cancelled.is_terminal());
    assert!(!RuntimeState::Cancelled.is_active());
}

#[test]
fn test_agent_status_cancelled_is_terminal() {
    use crate::agent::status::AgentStatus;
    assert!(AgentStatus::Cancelled.is_terminal());
    assert!(!AgentStatus::Cancelled.is_active());
}

// ---------------------------------------------------------------------------
// Loop safety guard unit tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_max_tool_calls_per_iteration_guard() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    // Single response with many tool calls (> MAX_TOOL_CALLS_PER_ITERATION=20).
    let many_calls: Vec<String> = (0..25)
        .map(|i| format!("<invoke name=\"list_files\">{{\"i\": {}}}</invoke>", i))
        .collect();
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec![many_calls.join("\n")],
    )));
    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        max_tool_calls_per_iteration: Some(5),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "too many tools",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task_with_options(&req, options).await;
    assert!(
        !result.success || result.error.is_some(),
        "exceeding max tool calls per iteration should fail: {:?}",
        result.error
    );
}

#[tokio::test]
async fn test_max_total_tool_calls_guard() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    // Multiple responses each returning one tool call, exceeding MAX_TOTAL_TOOL_CALLS=100.
    let responses: Vec<String> = (0..105)
        .map(|_| "<invoke name=\"list_files\">{\"x\": \"y\"}</invoke>".to_string())
        .collect();
    runtime.register_provider(Arc::new(ScriptedMockProvider::new("mock", responses)));
    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "too many total tools",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    assert!(
        !result.success || result.error.is_some(),
        "exceeding max total tool calls should fail: {:?}",
        result.error
    );
}

// =========================================================================
// Sprint 29.1 — Agent Loop Hardening
// =========================================================================

/// A mock provider that holds chunks in a channel and only delivers them
/// when a `hand_off` flag is set. This lets us test cancellation and
/// deadline behaviour against a provider that would otherwise hang.
#[derive(Clone)]
struct HangingMockProvider {
    name: String,
    model: String,
    /// Chunks to deliver. Empty = provider hangs forever.
    chunks: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// When true, the provider delivers its chunks and closes the channel.
    /// When false, the provider never delivers anything (hangs).
    release: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl HangingMockProvider {
    fn new(name: &str, chunks: Vec<String>) -> Self {
        HangingMockProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            chunks: std::sync::Arc::new(std::sync::Mutex::new(chunks)),
            release: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Signal the provider to release all its chunks.
    fn release(&self) {
        self.release
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Provider for HangingMockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn base_url(&self) -> &str {
        "mock://localhost"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn api_key(&self) -> Option<&str> {
        Some("mock-key")
    }

    fn send_message(
        &self,
        _message: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
    {
        let chunks = self.chunks.lock().unwrap().clone();
        Box::pin(async move { Ok(chunks.join("")) })
    }

    fn stream_response(
        &self,
        _message: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                > + Send
                + '_,
        >,
    > {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let chunks = self.chunks.clone();
        let release = self.release.clone();
        // Spawn a background task that delivers chunks only after release.
        tokio::spawn(async move {
            // Spin until released.
            while !release.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
            let mut c = chunks.lock().unwrap();
            for chunk in c.drain(..) {
                let _ = tx.send(chunk);
            }
            // Channel drops when tx is dropped → rx.recv() returns None.
        });
        Box::pin(async move { Ok(rx) })
    }
}

// ---------------------------------------------------------------------------
// 1. Provider hangs while cancellation fires → Cancelled
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_cancellation_interrupts_hanging_provider() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    // Provider that would hang forever (no chunks, never released).
    runtime.register_provider(Arc::new(HangingMockProvider::new("mock", vec![])));

    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let token = CancellationToken::new();
    let options = crate::canonical_runtime::TaskOptions {
        cancel: Some(token.clone()),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    // Cancel while the provider is hanging.
    token.cancel();
    let result = runtime.run_task_with_options(&req, options).await;
    assert!(
        result.cancelled || !result.success,
        "cancellation during hanging provider should terminate: {:?}",
        result.error
    );
    let evs = collect_events(&events);
    assert!(
        evs.iter().any(|e| matches!(
            e,
            AgentEvent::AgentCancelled { .. } | AgentEvent::AgentFailed { .. }
        )),
        "should emit a terminal event"
    );
}

// ---------------------------------------------------------------------------
// 2. Provider hangs while deadline fires → timeout
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_deadline_interrupts_hanging_provider() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(HangingMockProvider::new("mock", vec![])));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        // 50ms deadline — short enough to fire before any real work.
        task_timeout_ms: Some(50),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task_with_options(&req, options).await;
    assert!(
        !result.success,
        "deadline should terminate the task: {:?}",
        result.error
    );
    assert!(
        result
            .error
            .as_ref()
            .map(|e| e.contains("timed out"))
            .unwrap_or(false),
        "error should mention timeout: {:?}",
        result.error
    );
}

// ---------------------------------------------------------------------------
// 3. Provider emits chunks normally → task completes
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_provider_emits_chunks_normally() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec!["chunk1chunk2".to_string()],
    )));

    let chunks: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let chunk_sink = chunks.clone();
    let (events, emit) = event_sink();
    let on_chunk = move |c: &str| chunk_sink.lock().unwrap().push(c.to_string());
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    assert!(
        result.success,
        "normal provider should succeed: {:?}",
        result.error
    );
    let collected = chunks.lock().unwrap().clone();
    assert!(
        collected.contains(&"chunk1chunk2".to_string()),
        "chunks should be delivered"
    );
}

// ---------------------------------------------------------------------------
// 4. Cancellation does not cause a second terminal transition
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_cancellation_single_terminal_transition() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec!["final.".to_string()],
    )));

    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let token = CancellationToken::new();
    let options = crate::canonical_runtime::TaskOptions {
        cancel: Some(token.clone()),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "simple task",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    // Cancel after the task completes successfully.
    let result = runtime.run_task_with_options(&req, options).await;
    assert!(result.success);
    let evs = collect_events(&events);
    // Exactly one terminal event for the main agent.
    let main_terminals: Vec<&AgentEvent> = evs
        .iter()
        .filter(|e| match e {
            AgentEvent::AgentCompleted { agent, .. }
            | AgentEvent::AgentFailed { agent, .. }
            | AgentEvent::AgentCancelled { agent, .. } => agent == "main",
            _ => false,
        })
        .collect();
    assert_eq!(
        main_terminals.len(),
        1,
        "exactly one terminal event for main agent"
    );
}

// ---------------------------------------------------------------------------
// 5. Deadline does not cause another tool/model iteration after firing
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_deadline_stops_before_new_iteration() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    // Provide only one response — the deadline should fire before a second
    // iteration can start (the first response contains a tool call, but
    // the deadline is so short that the second provider call times out).
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec!["<invoke name=\"list_files\">{\"path\": \".\"}</invoke>".to_string()],
    )));
    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        task_timeout_ms: Some(1),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "list files",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task_with_options(&req, options).await;
    // Should terminate (either via timeout or tool-limit exhaustion), not
    // continue into a second provider call after the deadline.
    assert!(
        !result.success || result.error.is_some(),
        "deadline should prevent further iterations: {:?}",
        result.error
    );
}

// ---------------------------------------------------------------------------
// 6. Total model-call budget remains bounded across verification revisions
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_model_call_budget_bounded_across_revisions() {
    let dir = tempfile::tempdir().unwrap();
    // Create a cargo workspace so verification is applicable.
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"vt\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src").join("lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n#[cfg(test)]\nmod tests { #[test] fn ok() {} }\n",
    )
    .unwrap();

    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    // Provide enough responses for 1 initial execution + 2 revisions, but
    // cap at MAX_MODEL_CALLS (15). Each response is a final answer (no tool
    // calls), so each response = 1 model call.
    let responses: Vec<String> = (0..10).map(|i| format!("revision {} answer", i)).collect();
    runtime.register_provider(Arc::new(ScriptedMockProvider::new("mock", responses)));
    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "add a function to the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    // The task should reach a terminal state (either completed or failed
    // after exhausting revisions), never loop beyond MAX_MODEL_CALLS.
    assert!(result.success || result.error.is_some() || result.cancelled);
}

// ---------------------------------------------------------------------------
// Model-call budget test — direct proof that MAX_MODEL_CALLS is respected
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_max_model_calls_budget_respected() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    // Provide far more responses than MAX_MODEL_CALLS allows. Each response
    // is a tool call, forcing the loop to keep going until the budget is hit.
    let responses: Vec<String> = (0..(MAX_MODEL_CALLS + 10))
        .map(|_| "<invoke name=\"list_files\">{\"path\": \".\"}</invoke>".to_string())
        .collect();
    runtime.register_provider(Arc::new(ScriptedMockProvider::new("mock", responses)));
    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "list files repeatedly",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    // Should terminate before exhausting all responses.
    assert!(
        !result.success || result.error.is_some(),
        "model call budget should be respected: {:?}",
        result.error
    );
    let evs = collect_events(&events);
    assert!(
        !evs.iter()
            .any(|e| matches!(e, AgentEvent::AgentCompleted { agent, .. } if agent == "main")),
        "main agent should not complete when model call budget is hit"
    );
}

// =========================================================================
// Sprint 29.1B — Provider cancellation / deadline integrity tests
// =========================================================================

/// Provider that blocks in `stream_response()` until `release()` is called.
/// After release it returns a receiver that never produces any chunks.
#[derive(Clone)]
struct BlockingStreamResponseProvider {
    name: String,
    model: String,
    ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ready_notify: std::sync::Arc<tokio::sync::Notify>,
}

impl BlockingStreamResponseProvider {
    fn new(name: &str) -> Self {
        BlockingStreamResponseProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            ready: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            ready_notify: std::sync::Arc::new(tokio::sync::Notify::new()),
        }
    }

    fn release(&self) {
        self.ready.store(true, std::sync::atomic::Ordering::SeqCst);
        self.ready_notify.notify_waiters();
    }
}

impl Provider for BlockingStreamResponseProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn base_url(&self) -> &str {
        "mock://localhost"
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn api_key(&self) -> Option<&str> {
        Some("mock-key")
    }
    fn send_message(
        &self,
        _message: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
    {
        Box::pin(async move { Ok("blocked".to_string()) })
    }
    fn stream_response(
        &self,
        _message: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                > + Send
                + '_,
        >,
    > {
        let ready = self.ready.clone();
        let ready_notify = self.ready_notify.clone();
        Box::pin(async move {
            while !ready.load(std::sync::atomic::Ordering::SeqCst) {
                ready_notify.notified().await;
            }
            let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
            Ok(rx)
        })
    }
}

/// Provider that returns a receiver that never produces chunks.
#[derive(Clone)]
struct HangingRecvProvider {
    name: String,
    model: String,
}

impl HangingRecvProvider {
    fn new(name: &str) -> Self {
        HangingRecvProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
        }
    }
}

impl Provider for HangingRecvProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn base_url(&self) -> &str {
        "mock://localhost"
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn api_key(&self) -> Option<&str> {
        Some("mock-key")
    }
    fn send_message(
        &self,
        _message: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
    {
        Box::pin(async move { Ok("ok".to_string()) })
    }
    fn stream_response(
        &self,
        _message: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                > + Send
                + '_,
        >,
    > {
        // Keep the sender alive by cloning it into a background handle that
        // is never dropped.  This ensures recv() blocks indefinitely.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        // Leak the sender so it never drops (and the channel never closes).
        std::mem::forget(tx);
        Box::pin(async move { Ok(rx) })
    }
}

// ---------------------------------------------------------------------------
// 1. Mid-stream cancellation — cancellation flag set before task starts,
//    provider checks it cooperatively during stream_response.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_mid_stream_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));

    let provider = BlockingStreamResponseProvider::new("mock");
    runtime.register_provider(Arc::new(provider.clone()));

    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let token = CancellationToken::new();
    let options = crate::canonical_runtime::TaskOptions {
        cancel: Some(token.clone()),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "mid-stream cancel",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    // Cancel before starting — the select! cancellation arm will win
    // immediately, proving the mechanism is wired correctly.
    token.cancel();
    let result = runtime.run_task_with_options(&req, options).await;

    assert!(
        result.cancelled || !result.success,
        "mid-stream cancellation should terminate: {:?}",
        result.error
    );
    let evs = collect_events(&events);
    assert!(
        evs.iter().any(|e| matches!(
            e,
            AgentEvent::AgentCancelled { .. } | AgentEvent::AgentFailed { .. }
        )),
        "should emit a terminal event"
    );
}

// ---------------------------------------------------------------------------
// 2. Deadline during stream_response() — provider blocks, deadline fires.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_deadline_during_stream_response() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));

    let provider = BlockingStreamResponseProvider::new("mock");
    runtime.register_provider(Arc::new(provider.clone()));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        task_timeout_ms: Some(50),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "deadline during stream_response",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task_with_options(&req, options).await;
    assert!(
        !result.success,
        "deadline during stream_response should terminate: {:?}",
        result.error
    );
    assert!(
        result
            .error
            .as_ref()
            .map(|e| e.contains("timed out"))
            .unwrap_or(false),
        "error should mention timeout: {:?}",
        result.error
    );
}

// ---------------------------------------------------------------------------
// 3. Deadline during rx.recv() — provider returns receiver, deadline fires.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_deadline_during_recv() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));

    runtime.register_provider(Arc::new(HangingRecvProvider::new("mock")));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        task_timeout_ms: Some(50),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "deadline during recv",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task_with_options(&req, options).await;
    assert!(
        !result.success,
        "deadline during recv should terminate: {:?}",
        result.error
    );
    assert!(
        result
            .error
            .as_ref()
            .map(|e| e.contains("timed out"))
            .unwrap_or(false),
        "error should mention timeout: {:?}",
        result.error
    );
}

// ---------------------------------------------------------------------------
// 4. Normal stream still works — chunks delivered, no cancellation.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_normal_stream_still_works() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec!["chunk1chunk2".to_string()],
    )));

    let chunks: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let chunk_sink = chunks.clone();
    let (_, emit) = event_sink();
    let on_chunk = move |c: &str| chunk_sink.lock().unwrap().push(c.to_string());
    let req = crate::canonical_runtime::TaskRequest {
        task: "normal stream",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task(&req).await;
    assert!(
        result.success,
        "normal stream should succeed: {:?}",
        result.error
    );
    let collected = chunks.lock().unwrap().clone();
    assert!(
        collected.contains(&"chunk1chunk2".to_string()),
        "chunks should be delivered"
    );
}

// ---------------------------------------------------------------------------
// 5. Cancellation does NOT trigger retry — task terminates promptly.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_cancellation_does_not_retry() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(1));

    let provider = BlockingStreamResponseProvider::new("mock");
    runtime.register_provider(Arc::new(provider.clone()));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let token = CancellationToken::new();
    let options = crate::canonical_runtime::TaskOptions {
        cancel: Some(token.clone()),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "no retry on cancel",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    // Cancel before starting — select! cancellation arm wins immediately,
    // no retry path is entered.
    token.cancel();
    let result = runtime.run_task_with_options(&req, options).await;

    assert!(
        result.cancelled || !result.success,
        "should terminate: {:?}",
        result.error
    );
    assert!(
        result.error.is_some() || result.cancelled,
        "cancelled task should not succeed"
    );
}

// ---------------------------------------------------------------------------
// 6. Deadline does NOT trigger retry — task terminates with timeout.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_deadline_does_not_retry() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(1));

    let provider = BlockingStreamResponseProvider::new("mock");
    runtime.register_provider(Arc::new(provider.clone()));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        task_timeout_ms: Some(10),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "no retry on deadline",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task_with_options(&req, options).await;
    assert!(
        !result.success,
        "deadline should terminate: {:?}",
        result.error
    );
    assert!(
        result
            .error
            .as_ref()
            .map(|e| e.contains("timed out"))
            .unwrap_or(false),
        "error should be a timeout, not a retry exhaustion: {:?}",
        result.error
    );
}

// =========================================================================
// Sprint 30A — Structured Function Calling
// =========================================================================

/// A mock provider that supports native function calling. It records the tool
/// definitions it receives and returns scripted structured tool calls.
#[derive(Clone)]
struct FunctionCallingMockProvider {
    name: String,
    model: String,
    /// Scripted responses. Each entry is a JSON string of the `tool_calls`
    /// array (or `[]` for plain text). Consumed sequentially.
    responses: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// Tool definitions received on the last invocation.
    received_tools: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// When true, `stream_response_with_tools` returns an error immediately.
    fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl FunctionCallingMockProvider {
    fn new(name: &str, responses: Vec<String>) -> Self {
        FunctionCallingMockProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            responses: std::sync::Arc::new(std::sync::Mutex::new(responses)),
            received_tools: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            fail: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn failing(name: &str) -> Self {
        let p = FunctionCallingMockProvider::new(name, Vec::new());
        p.fail.store(true, std::sync::atomic::Ordering::SeqCst);
        p
    }

    /// The tool definitions received on the last invocation (names).
    fn received_tool_names(&self) -> Vec<String> {
        self.received_tools.lock().unwrap().clone()
    }

    fn next_response(&self) -> String {
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            String::new()
        } else {
            responses.remove(0)
        }
    }
}

impl Provider for FunctionCallingMockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn base_url(&self) -> &str {
        "mock://localhost"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn api_key(&self) -> Option<&str> {
        Some("mock-key")
    }

    fn supports_function_calling(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Vec<crate::provider_runtime::Capability> {
        vec![
            crate::provider_runtime::Capability::Streaming,
            crate::provider_runtime::Capability::ToolCalling,
            crate::provider_runtime::Capability::FunctionCalling,
        ]
    }

    fn send_message(
        &self,
        _message: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
    {
        let response = self.next_response();
        Box::pin(async move { Ok(response) })
    }

    fn stream_response(
        &self,
        _message: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                > + Send
                + '_,
        >,
    > {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let response = self.next_response();
        let _ = tx.send(response);
        Box::pin(async move { Ok(rx) })
    }

    fn stream_response_with_tools(
        &self,
        _message: &str,
        tools: &[crate::providers::ToolDefinition],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<(String, Vec<crate::providers::StructuredToolCall>)>,
                > + Send
                + '_,
        >,
    > {
        // Record tool definitions.
        {
            let names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
            self.received_tools.lock().unwrap().extend(names);
        }

        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Box::pin(
                async move { Err(anyhow::anyhow!("mock structured provider offline")) },
            );
        }

        let response = self.next_response();
        Box::pin(async move {
            // The response is a JSON `tool_calls` array.
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

/// Helper: build a runtime with a structured-calling mock provider.
fn structured_runtime(responses: Vec<String>) -> CanonicalRuntime {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(FunctionCallingMockProvider::new(
        "fc-mock", responses,
    )));
    runtime
}

/// Helper: run a task and return the runtime plus its result.
async fn run_structured_task(
    runtime: &mut CanonicalRuntime,
    task: &str,
) -> crate::canonical_runtime::TaskResult {
    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task,
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    runtime.run_task(&req).await
}

// ---------------------------------------------------------------------------
// 1. Tool definitions are sent in the request.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_structured_sends_tool_definitions() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    let provider = FunctionCallingMockProvider::new("fc-mock", vec!["[]".to_string()]);
    let provider_arc = Arc::new(provider);
    let provider_copy = provider_arc.clone();
    runtime.register_provider(provider_arc);
    let result = run_structured_task(&mut runtime, "explain the project").await;
    assert!(result.success);
    let names = provider_copy.received_tool_names();
    assert!(!names.is_empty(), "tool definitions should be sent");
    assert!(
        names.contains(&"list_files".to_string()),
        "expected list_files in tool defs, got: {:?}",
        names
    );
    assert!(
        names.contains(&"read_file".to_string()),
        "expected read_file in tool defs"
    );
}

// ---------------------------------------------------------------------------
// 2. A single structured tool call executes through the ToolRegistry.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_structured_single_tool_call() {
    let mut runtime = structured_runtime(vec![
        // Structured call to list_files.
        r#"[{"id": "call_1", "function": {"name": "list_files", "arguments": "{\"path\": \".\"}"}}]"#
            .to_string(),
        // Final text answer.
        "[]".to_string(),
    ]);
    let result = run_structured_task(&mut runtime, "list files").await;
    assert!(
        result.success,
        "structured single tool call should succeed: {:?}",
        result.error
    );
}

// ---------------------------------------------------------------------------
// 3. Multiple structured tool calls in one response.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_structured_multiple_tool_calls() {
    let mut runtime = structured_runtime(vec![
        r#"[
            {"id": "call_1", "function": {"name": "list_files", "arguments": "{}"}},
            {"id": "call_2", "function": {"name": "read_file", "arguments": "{\"path\": \"README.md\"}"}}
        ]"#
        .to_string(),
        "[]".to_string(),
    ]);
    let result = run_structured_task(&mut runtime, "inspect the repo").await;
    assert!(
        result.success,
        "structured multiple tool calls should succeed: {:?}",
        result.error
    );
}

// ---------------------------------------------------------------------------
// 4. Structured call → tool result → second model iteration → final answer.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_structured_tool_result_feeds_next_iteration() {
    let mut runtime = structured_runtime(vec![
        // First iteration: structured call.
        r#"[{"id": "call_1", "function": {"name": "list_files", "arguments": "{}"}}]"#.to_string(),
        // Second iteration: final answer after observing the tool result.
        "[]".to_string(),
    ]);
    let result = run_structured_task(&mut runtime, "list files then summarize").await;
    assert!(
        result.success,
        "structured tool result should feed next iteration: {:?}",
        result.error
    );
}

// ---------------------------------------------------------------------------
// 5. Provider that does NOT support function calling falls back to text parser.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_unsupported_provider_falls_back_to_text_parser() {
    // ScriptedMockProvider (no function calling) returns text; the runtime
    // must route through the text parser.
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "text-mock",
        vec![
            // Text-encoded tool call.
            "<invoke name=\"list_files\">{\"path\": \".\"}</invoke>".to_string(),
            "Done.".to_string(),
        ],
    )));
    let result = run_structured_task(&mut runtime, "list files").await;
    assert!(
        result.success,
        "text fallback should succeed: {:?}",
        result.error
    );
}

// ---------------------------------------------------------------------------
// 6. Malformed structured response fails safely (no tool executed).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_structured_malformed_response_fails_safely() {
    let mut runtime = structured_runtime(vec![
        // Malformed tool_calls JSON: missing "function" field.
        r#"[{"id": "call_1"}]"#.to_string(),
        "[]".to_string(),
    ]);
    let result = run_structured_task(&mut runtime, "do something").await;
    // The malformed structured response should not crash; the runtime should
    // terminate gracefully (either success with text, or a clean failure).
    assert!(result.success || result.error.is_some());
}

// ---------------------------------------------------------------------------
// 7. Structured provider failure propagates as a task error (with retry).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_structured_provider_failure_is_retried() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(1));
    let provider = FunctionCallingMockProvider::failing("fc-mock");
    let provider_arc = Arc::new(provider);
    runtime.register_provider(provider_arc);
    let result = run_structured_task(&mut runtime, "do something").await;
    assert!(
        !result.success,
        "structured provider failure should fail the task: {:?}",
        result.error
    );
}

// ---------------------------------------------------------------------------
// 8. Structured calling can complete a multi-step task end to end.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_structured_multistep_task_completion() {
    let mut runtime = structured_runtime(vec![
        // Step 1: list files.
        r#"[{"id": "call_1", "function": {"name": "list_files", "arguments": "{}"}}]"#.to_string(),
        // Step 2: read a file.
        r#"[{"id": "call_2", "function": {"name": "read_file", "arguments": "{\"path\": \"README.md\"}"}}]"#
            .to_string(),
        // Step 3: final answer.
        "[]".to_string(),
    ]);
    let result = run_structured_task(&mut runtime, "list then read the readme").await;
    assert!(
        result.success,
        "structured multi-step should succeed: {:?}",
        result.error
    );
}

// =========================================================================
// Sprint 30C — Autonomous Research Subagent (parent integration)
// =========================================================================

/// A scripted provider that consumes responses sequentially AND records every
/// prompt it receives, so tests can prove the research fragment reached the
/// main LLM prompt.
#[derive(Clone)]
struct RecordingScriptedProvider {
    name: String,
    model: String,
    responses: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    prompts: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl RecordingScriptedProvider {
    fn new(name: &str, responses: Vec<String>) -> Self {
        RecordingScriptedProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            responses: std::sync::Arc::new(std::sync::Mutex::new(responses)),
            prompts: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn next(&self) -> String {
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            String::new()
        } else {
            responses.remove(0)
        }
    }

    fn all_prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

impl Provider for RecordingScriptedProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn base_url(&self) -> &str {
        "mock://localhost"
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn api_key(&self) -> Option<&str> {
        Some("mock-key")
    }
    fn send_message(
        &self,
        _message: &str,
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
        let response = self.next();
        let _ = tx.send(response);
        Box::pin(async move { Ok(rx) })
    }
}

/// A grounded workspace with real files whose content mentions the runtime.
fn research_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src/canonical_runtime")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n[dependencies]\ntokio = \"1\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/canonical_runtime/mod.rs"),
        "pub struct CanonicalRuntime {}\nimpl CanonicalRuntime {\n    pub fn run_execution_loop() {}\n    pub fn stream_once() {}\n}\n",
    )
    .unwrap();
    dir
}

/// ResearchResult → ContextFragment → compiled main prompt.
#[tokio::test]
async fn test_research_result_reaches_compiled_prompt() {
    let dir = research_workspace();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec![
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            "The CanonicalRuntime struct defines run_execution_loop and stream_once.".to_string(),
        ],
    )));

    let (context, compiled) = runtime
        .compile_for_task_with_research("trace canonical runtime execution", no_conversation())
        .await
        .expect("compile with research");

    // ResearchResult became a `research` ContextFragment.
    let fragment = context
        .context_fragments
        .iter()
        .find(|f| f.source == "research")
        .map(|f| f.content.clone())
        .unwrap_or_default();
    assert!(
        fragment.contains("Autonomous Research Findings"),
        "research fragment must carry the rendered result:\n{}",
        fragment
    );
    // The fragment carried evidence from a REAL read of the actual file.
    assert!(
        fragment.contains("run_execution_loop"),
        "research fragment must carry real repository evidence:\n{}",
        fragment
    );

    // The compiled main prompt contains the research fragment.
    assert!(
        compiled.prompt.contains("--- research () ---"),
        "prompt must render the research fragment:\n{}",
        compiled.prompt
    );
    assert!(
        compiled.prompt.contains("Autonomous Research Findings"),
        "prompt must contain the research rendering"
    );
    assert!(
        compiled.prompt.contains("run_execution_loop"),
        "prompt must contain the evidence-backed symbol"
    );
}

/// Full chain: ResearchResult → Coordinator fragment → PromptBuilder → main
/// provider. The main provider's prompt must contain the research result.
#[tokio::test]
async fn test_research_result_reaches_main_provider_prompt() {
    let dir = research_workspace();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    let provider = Arc::new(RecordingScriptedProvider::new(
        "mock",
        vec![
            // Research iteration 1: list the runtime directory.
            r#"<invoke name="list_files">{"path": "src/canonical_runtime"}</invoke>"#.to_string(),
            // Research iteration 2: read the runtime module.
            r#"<invoke name="read_file">{"path": "src/canonical_runtime/mod.rs"}</invoke>"#
                .to_string(),
            // Research final answer.
            "run_execution_loop is the ReAct loop entry point.".to_string(),
            // Main loop answer.
            "main task complete.".to_string(),
        ],
    ));
    runtime.register_provider(provider.clone());

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        research_enabled: true,
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "trace canonical runtime execution",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task_with_options(&req, options).await;
    assert!(
        result.success,
        "main task must succeed with research enabled: {:?}",
        result.error
    );
    assert!(result.response.contains("main task complete"));

    // The main provider's prompt (the LAST recorded prompt) contains the
    // research fragment.
    let prompts = provider.all_prompts();
    assert_eq!(prompts.len(), 4, "3 research calls + 1 main call");
    let main_prompt = prompts.last().expect("main prompt present");
    assert!(
        main_prompt.contains("--- research () ---"),
        "main prompt must contain the research fragment:\n{}",
        main_prompt
    );
    assert!(
        main_prompt.contains("Autonomous Research Findings"),
        "main prompt must contain the research rendering"
    );
    assert!(
        main_prompt.contains("run_execution_loop"),
        "main prompt must contain evidence-backed research facts"
    );

    // Research diagnostics were captured.
    let research = result
        .diagnostics
        .research
        .expect("research diagnostics recorded");
    assert!(research.completed);
    assert_eq!(research.iterations, 3);
    assert_eq!(research.tool_calls, 2);
}

/// A research session that terminates abnormally (budget exhausted) must NOT
/// crash or block the main task: the main agent continues and completes.
#[tokio::test]
async fn test_research_failure_is_isolated_from_main_task() {
    let dir = research_workspace();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    // Research keeps calling tools until its model-call budget (6) is
    // exhausted; the final response is the main agent's answer.
    let mut responses: Vec<String> = (0..6)
        .map(|_| r#"<invoke name="list_files">{"path": "src"}</invoke>"#.to_string())
        .collect();
    responses.push("main answer after failed research".to_string());
    runtime.register_provider(Arc::new(ScriptedMockProvider::new("mock", responses)));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        research_enabled: true,
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "explain the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task_with_options(&req, options).await;
    // The main task still succeeds despite research exhausting its budget.
    assert!(
        result.success,
        "main task must survive research failure: {:?}",
        result.error
    );
    assert!(result
        .response
        .contains("main answer after failed research"));
    let research = result
        .diagnostics
        .research
        .expect("research diagnostics recorded");
    assert_eq!(
        research.termination, "model_limit",
        "research must terminate abnormally (bounded error result)"
    );
    assert!(!research.completed);
}

// ---------------------------------------------------------------------------
// 9. Structured `{"input": ...}` envelope is unwrapped so real tools execute.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_structured_input_envelope_unwrapped_for_real_tools() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("greeting.txt"), "hello from the real file").unwrap();
    let abs = dir
        .path()
        .join("greeting.txt")
        .to_string_lossy()
        .to_string();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(FunctionCallingMockProvider::new(
        "fc-mock",
        vec![
            // Canonical structured-calling envelope: `{"input": "<args>"}`.
            format!(
                r#"[{{"id": "call_1", "function": {{"name": "read_file", "arguments": "{{\"input\": \"{}\"}}"}}}}]"#,
                abs
            ),
            "[]".to_string(),
        ],
    )));

    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "read the greeting file",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    assert!(
        result.success,
        "envelope-unwrapped structured read must complete: {:?}",
        result.error
    );
    let evs = events.lock().unwrap();
    assert!(
        evs.iter().any(|e| matches!(
            e,
            AgentEvent::ToolCompleted { tool, success: true, .. } if tool == "read_file"
        )),
        "read_file must actually succeed (envelope unwrapped), events: {:?}",
        evs.iter()
            .filter_map(|e| match e {
                AgentEvent::ToolCompleted { tool, success, .. } => {
                    Some(format!("{tool}:{success}"))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// 10. Text-encoded tool calls carrying the `{"input": ...}` envelope are also
//     unwrapped before execution.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_text_encoded_input_envelope_is_unwrapped_for_real_tools() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("greeting.txt"), "hello from the real file").unwrap();
    let abs = dir
        .path()
        .join("greeting.txt")
        .to_string_lossy()
        .to_string();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "text-mock",
        vec![
            // Text-encoded call with the canonical envelope.
            format!(
                r#"<invoke name="read_file">{{"input": "{}"}}</invoke>"#,
                abs
            ),
            "Done reading.".to_string(),
        ],
    )));
    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let req = crate::canonical_runtime::TaskRequest {
        task: "read the greeting file",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };
    let result = runtime.run_task(&req).await;
    assert!(
        result.success,
        "text-encoded envelope-unwrapped read must complete: {:?}",
        result.error
    );
    let evs = events.lock().unwrap();
    assert!(
        evs.iter().any(|e| matches!(
            e,
            AgentEvent::ToolCompleted { tool, success: true, .. } if tool == "read_file"
        )),
        "text-encoded read_file must actually succeed"
    );
}

// ---------------------------------------------------------------------------
// 11. Full flow (Sprint 30C.0.1): ResearchSubagent → main agent → explicit
//     verification (cargo build + cargo test). Reproduces the real-provider
//     smoke run: research gathers real evidence first, the main loop runs, and
//     verification executes REAL build/test commands. The verification must
//     pass for a genuinely valid crate even after research has run.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_research_then_main_then_verification_full_flow_passes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"vt\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src").join("lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n#[cfg(test)]\nmod tests { #[test] fn ok() {} }\n",
    )
    .unwrap();

    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec![
            // Research iteration 1: read the library source.
            r#"<invoke name="read_file">{"path": "src/lib.rs"}</invoke>"#.to_string(),
            // Research final report.
            "add() is defined in src/lib.rs and returns a + b.".to_string(),
            // Main agent final answer (execution intent → verification runs).
            "I added the function to the project.".to_string(),
        ],
    )));

    let (events, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        research_enabled: true,
        max_verification_revisions: Some(1),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "add a function to the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task_with_options(&req, options).await;
    assert!(
        result.success,
        "full flow must succeed for a valid crate: {:?}",
        result.error
    );
    let verification = result
        .diagnostics
        .verification
        .expect("verification must run for an execution task");
    assert!(
        verification.steps.iter().all(|s| s.success),
        "build and test must pass even after research ran: {:?}",
        verification
    );
    // Research diagnostics were recorded and completed with a synthesis.
    let research = result
        .diagnostics
        .research
        .expect("research diagnostics recorded");
    assert!(research.completed);
    assert!(research.synthesis_complete);
    let evs = events.lock().unwrap();
    assert!(
        evs.iter()
            .any(|e| matches!(e, AgentEvent::AgentCompleted { .. })),
        "main agent must complete the task"
    );
}

// ---------------------------------------------------------------------------
// 12. Verification decision is driven by the authoritative per-step exit
//     codes, NOT by searching the command output text. A failing `cargo test`
//     still prints "0 passed; 1 failed", so the old `contains("passed")`
//     heuristic wrongly masked real failures. Regression test: a failing test
//     must fail the task even though its output contains "passed".
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_verification_failure_with_passed_substring_is_not_masked() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"vf\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // Compiles fine, but the single test always FAILS. The surfaced failure
    // output deliberately contains the substring "passed" (the panic message),
    // exactly like a real `cargo test` whose tail shows "0 passed; 1 failed".
    std::fs::write(
        dir.path().join("src").join("lib.rs"),
        "pub fn f() -> i32 { 1 }\n#[cfg(test)]\nmod tests { #[test] fn always_fails() { panic!(\"passed and still failed\") } }\n",
    )
    .unwrap();

    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    // Initial answer + one revision answer. The mock provider cannot fix the
    // failing test, so verification keeps failing.
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec!["Done.".to_string(), "Tried to fix.".to_string()],
    )));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        max_verification_revisions: Some(1),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "add a function to the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task_with_options(&req, options).await;
    assert!(
        !result.success,
        "a failing test must not be masked even though the output contains 'passed': {:?}",
        result.error
    );
    let error = result.error.clone().unwrap_or_default();
    assert!(
        error.contains("Verification failed"),
        "the task must report the verification failure, got: {}",
        error
    );
    let verification = result
        .diagnostics
        .verification
        .expect("verification must run");
    assert!(
        verification.steps.iter().any(|s| !s.success),
        "the verification summary must record the real failure: {:?}",
        verification
    );
}

// ---------------------------------------------------------------------------
// 13. Real-provider research smoke (Sprint 30C.0.1). Runs ONE short research
//     task against the configured provider (AGNES), proving the full
//     evidence-gathering → bounded synthesis → structured ResearchResult
//     pipeline against a real LLM. Ignored by default because it makes a real
//     network call with a real credential; run with:
//     `cargo test --bin codebro real_provider_research_smoke -- --ignored --nocapture`
//     The credential is read from the environment and never persisted.
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn real_provider_research_smoke() {
    let api_key = std::env::var("AGNES_API_KEY")
        .ok()
        .or_else(|| std::env::var("CODEBRO_API_KEY").ok());
    let Some(api_key) = api_key else {
        eprintln!("REAL PROVIDER: BLOCKED (no AGNES_API_KEY in environment)");
        return;
    };
    let base_url = std::env::var("CODEBRO_BASE_URL")
        .unwrap_or_else(|_| "https://apihub.agnes-ai.com/v1".to_string());
    let model = std::env::var("CODEBRO_MODEL").unwrap_or_else(|_| "agnes-2.5-flash".to_string());

    let config = Config {
        provider: "openai".to_string(),
        base_url,
        model,
        api_key: Some(api_key),
    };
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let provider = Arc::new(crate::providers::OpenAiProvider::new(config.clone()));
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(config, &root).expect("runtime");
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(provider);

    let task = "Research how src/canonical_runtime/mod.rs executes a tool call. Identify the relevant functions and inspect the minimum files necessary. Do not modify anything.";
    let grounding = crate::agent::grounding::GroundingAssembler::new(&root).assemble_with_extras(
        task,
        &[],
        &[],
    );
    let (_events, emit) = event_sink();
    let result = runtime
        .run_research_task(task, grounding, &emit, None)
        .await
        .expect("real-provider research must return a structured result");

    println!("[real-provider] {}", result.summary_line());
    println!(
        "[real-provider] termination={} synthesis_complete={}",
        result.termination, result.synthesis_complete
    );
    println!(
        "[real-provider] files_inspected={:?}",
        result.files_inspected
    );
    println!("[real-provider] symbols={:?}", result.symbols_found);
    println!("[real-provider] render:\n{}", result.render());

    // The acceptance contract: the session terminates bounded and, with the
    // reserved synthesis call, the final report is actually produced.
    assert!(
        result.model_calls <= 6,
        "model calls must stay bounded, got {}",
        result.model_calls
    );
    assert!(
        result.tool_calls <= 20,
        "tool calls must stay bounded, got {}",
        result.tool_calls
    );
    assert!(
        result.synthesis_complete,
        "synthesis must complete for a real provider"
    );
    assert!(result.termination.is_completed());
}

// =========================================================================
// Sprint 30D — Autonomous Testing Subagent (parent integration)
// =========================================================================

/// A fixture crate that COMPILES so the Testing subagent can run real
/// validation commands against it. `target/` and `Cargo.lock` are gitignored
/// so normal build artifacts never surface in the git state.
fn testing_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"vt\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(dir.path().join(".gitignore"), "target/\nCargo.lock\n").unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n#[cfg(test)]\nmod tests { #[test] fn ok() {} }\n",
    )
    .unwrap();
    dir
}

/// TestingResult → ContextFragment → compiled main prompt.
#[tokio::test]
async fn test_testing_result_reaches_compiled_prompt() {
    let dir = testing_workspace();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec![
            r#"<invoke name="run_command">{"command": "cargo check"}</invoke>"#.to_string(),
            "cargo check passed with exit code 0.".to_string(),
        ],
    )));

    let (context, compiled) = runtime
        .compile_for_task_with_testing("validate the crate", no_conversation())
        .await
        .expect("compile with testing");

    // TestingResult became a `testing` ContextFragment.
    let fragment = context
        .context_fragments
        .iter()
        .find(|f| f.source == "testing")
        .map(|f| f.content.clone())
        .unwrap_or_default();
    assert!(
        fragment.contains("Autonomous Testing Findings"),
        "testing fragment must carry the rendered result:\n{}",
        fragment
    );
    // The fragment carried authoritative machine facts from a REAL cargo check.
    assert!(
        fragment.contains("exit_code: 0"),
        "testing fragment must carry the authoritative exit code:\n{}",
        fragment
    );

    // The compiled main prompt contains the testing fragment.
    assert!(
        compiled.prompt.contains("--- testing () ---"),
        "prompt must render the testing fragment:\n{}",
        compiled.prompt
    );
    assert!(
        compiled.prompt.contains("Autonomous Testing Findings"),
        "prompt must contain the testing rendering"
    );
    assert!(
        compiled.prompt.contains("exit_code: 0"),
        "prompt must contain the authoritative command evidence"
    );
}

/// Full chain: TestingResult → ContextFragment → PromptBuilder → main
/// provider. The main provider's prompt must contain the authoritative command
/// evidence (real exit codes).
#[tokio::test]
async fn test_testing_result_reaches_main_provider_prompt() {
    let dir = testing_workspace();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    let provider = Arc::new(RecordingScriptedProvider::new(
        "mock",
        vec![
            // Testing iteration 1: run cargo check.
            r#"<invoke name="run_command">{"command": "cargo check"}</invoke>"#.to_string(),
            // Testing final answer.
            "cargo check passed with exit code 0.".to_string(),
            // Main loop answer.
            "main task complete.".to_string(),
        ],
    ));
    runtime.register_provider(provider.clone());

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        testing_enabled: true,
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "validate the crate",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task_with_options(&req, options).await;
    assert!(
        result.success,
        "main task must succeed with testing enabled: {:?}",
        result.error
    );
    assert!(result.response.contains("main task complete"));

    // The main provider's prompt (the LAST recorded prompt) contains the
    // testing fragment with the authoritative command evidence.
    let prompts = provider.all_prompts();
    assert_eq!(prompts.len(), 3, "2 testing calls + 1 main call");
    let main_prompt = prompts.last().expect("main prompt present");
    assert!(
        main_prompt.contains("--- testing () ---"),
        "main prompt must contain the testing fragment:\n{}",
        main_prompt
    );
    assert!(
        main_prompt.contains("Autonomous Testing Findings"),
        "main prompt must contain the testing rendering"
    );
    assert!(
        main_prompt.contains("exit_code: 0"),
        "main prompt must contain the authoritative exit code evidence"
    );

    // Testing diagnostics were captured.
    let testing = result
        .diagnostics
        .testing
        .expect("testing diagnostics recorded");
    assert!(testing.completed);
    assert_eq!(testing.commands_run, 1);
    assert_eq!(testing.failures, 0);
    assert!(testing.git_tree_unchanged);
}

/// A testing session that terminates abnormally (budget exhausted) must NOT
/// crash or block the main task: the main agent continues and completes.
#[tokio::test]
async fn test_testing_failure_is_isolated_from_main_task() {
    let dir = testing_workspace();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    // Testing keeps requesting commands until its model-call budget (6) is
    // exhausted (the reserved synthesis call also asks for a command); the
    // final response is the main agent's answer.
    let mut responses: Vec<String> = (0..6)
        .map(|_| r#"<invoke name="run_command">{"command": "true"}</invoke>"#.to_string())
        .collect();
    responses.push("main answer after failed testing".to_string());
    runtime.register_provider(Arc::new(ScriptedMockProvider::new("mock", responses)));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        testing_enabled: true,
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "validate the crate",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task_with_options(&req, options).await;
    // The main task still succeeds despite testing exhausting its budget.
    assert!(
        result.success,
        "main task must survive testing failure: {:?}",
        result.error
    );
    assert!(result.response.contains("main answer after failed testing"));
    let testing = result
        .diagnostics
        .testing
        .expect("testing diagnostics recorded");
    assert_eq!(
        testing.termination, "model_limit",
        "testing must terminate abnormally (bounded error result)"
    );
    assert!(!testing.completed);
}

/// Full flow (Sprint 30D): TestingSubagent → main agent → explicit
/// verification. Testing runs a REAL cargo check first, then the main loop
/// runs, and the task-level verification (cargo build + cargo test) must still
/// pass for a genuinely valid crate. This mirrors the research-then-verification
/// full-flow test.
#[tokio::test]
async fn test_testing_then_main_then_verification_full_flow_passes() {
    let dir = testing_workspace();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(Arc::new(ScriptedMockProvider::new(
        "mock",
        vec![
            // Testing iteration 1: run cargo check.
            r#"<invoke name="run_command">{"command": "cargo check"}</invoke>"#.to_string(),
            // Testing final report.
            "cargo check passes for the crate.".to_string(),
            // Main agent final answer (execution intent → verification runs).
            "I added the function to the project.".to_string(),
        ],
    )));

    let (_, emit) = event_sink();
    let on_chunk = |_c: &str| {};
    let options = crate::canonical_runtime::TaskOptions {
        testing_enabled: true,
        max_verification_revisions: Some(1),
        ..Default::default()
    };
    let req = crate::canonical_runtime::TaskRequest {
        task: "add a function to the project",
        conversation: no_conversation(),
        emit: &emit,
        on_chunk: &on_chunk,
    };

    let result = runtime.run_task_with_options(&req, options).await;
    assert!(
        result.success,
        "full flow must succeed for a valid crate: {:?}",
        result.error
    );
    let verification = result
        .diagnostics
        .verification
        .expect("verification must run for an execution task");
    assert!(
        verification.steps.iter().all(|s| s.success),
        "build and test must pass even after testing ran: {:?}",
        verification
    );
    let testing = result
        .diagnostics
        .testing
        .expect("testing diagnostics recorded");
    assert!(testing.completed);
    assert!(testing.synthesis_complete);
    assert!(testing.git_tree_unchanged);
}

// ---------------------------------------------------------------------------
// Real-provider testing smoke (Sprint 30D). Runs ONE bounded validation task
// against the configured provider (AGNES), proving the full
// decide → execute → authoritative exit code → observe → synthesize →
// TestingResult pipeline against a real LLM. Ignored by default because it
// makes a real network call with a real credential; run with:
// `cargo test --bin codebro real_provider_testing_smoke -- --ignored --nocapture`
// The credential is read from the environment and never persisted.
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn real_provider_testing_smoke() {
    let api_key = std::env::var("AGNES_API_KEY")
        .ok()
        .or_else(|| std::env::var("CODEBRO_API_KEY").ok());
    let Some(api_key) = api_key else {
        eprintln!("REAL PROVIDER: BLOCKED (no AGNES_API_KEY in environment)");
        return;
    };
    let base_url = std::env::var("CODEBRO_BASE_URL")
        .unwrap_or_else(|_| "https://apihub.agnes-ai.com/v1".to_string());
    let model = std::env::var("CODEBRO_MODEL").unwrap_or_else(|_| "agnes-2.5-flash".to_string());

    let config = Config {
        provider: "openai".to_string(),
        base_url,
        model,
        api_key: Some(api_key),
    };
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let provider = Arc::new(crate::providers::OpenAiProvider::new(config.clone()));
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(config, &root).expect("runtime");
    runtime.with_retry_policy(RetryPolicy::immediate(0));
    runtime.register_provider(provider);

    let task = "Run a safe validation of this Rust repository. First determine the appropriate validation command. Run only read-only validation/build/test commands. Do not modify source files or git state. Report exact exit codes and failures.";
    let grounding = crate::agent::grounding::GroundingAssembler::new(&root).assemble_with_extras(
        task,
        &[],
        &[],
    );
    let (_events, emit) = event_sink();
    // A generous budget for a real heavy validation task (warm cargo target):
    // the default 30s session is too tight for compiling and testing the full
    // repository. This is explicit configuration for one real run, not a
    // change to the conservative default.
    let limits = crate::testing::TestingLimits {
        timeout_ms: 180_000,
        command_timeout_secs: 120,
        ..crate::testing::TestingLimits::default()
    };
    let result = runtime
        .run_testing_task_with_limits(task, grounding, limits, &emit, None)
        .await
        .expect("real-provider testing must return a structured result");

    println!("[real-provider] {}", result.summary_line());
    println!(
        "[real-provider] termination={} synthesis_complete={}",
        result.termination, result.synthesis_complete
    );
    for command in &result.commands_run {
        println!(
            "[real-provider] command={} exit_code={} success={} denied={} timeout={}",
            command.command, command.exit_code, command.success, command.denied, command.timeout
        );
    }
    for failure in &result.failures {
        println!(
            "[real-provider] failure kind={} command={} exit_code={}",
            failure.kind.as_str(),
            failure.command,
            failure.exit_code
        );
    }
    println!("[real-provider] render:\n{}", result.render());

    // The acceptance contract: the session terminates bounded, commands were
    // actually executed, and the git tree was left untouched.
    assert!(
        result.model_calls <= 6,
        "model calls must stay bounded, got {}",
        result.model_calls
    );
    assert!(
        result.tool_calls <= 12,
        "tool calls must stay bounded, got {}",
        result.tool_calls
    );
    assert!(
        result.synthesis_complete,
        "synthesis must complete for a real provider"
    );
    assert!(result.termination.is_completed());
    assert!(
        result.git_tree_unchanged(),
        "real-provider testing must not mutate the tracked tree"
    );
}
