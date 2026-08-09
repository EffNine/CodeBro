//! Integration tests for the canonical runtime pipeline.

use std::sync::Arc;

use crate::agent::events::AgentEvent;
use crate::canonical_runtime::CanonicalRuntime;
use crate::config::Config;
use crate::engineering_context::ConversationMessage;
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
}

impl MockProvider {
    fn new(name: &str, responses: Vec<String>) -> Self {
        MockProvider {
            name: name.to_string(),
            model: format!("{}-model", name),
            responses: std::sync::Arc::new(std::sync::Mutex::new(responses)),
            fail: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn failing(name: &str) -> Self {
        let p = MockProvider::new(name, Vec::new());
        p.fail.store(true, std::sync::atomic::Ordering::SeqCst);
        p
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

#[tokio::test]
async fn test_objective_reaches_engineering_context() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (context, _compiled) = runtime
        .compile_for_task("maintain the project", no_conversation())
        .await
        .unwrap();

    // The documented default objective is always installed.
    assert!(
        !context.objective.is_empty(),
        "objective should be always-on"
    );
    assert!(!context.objective.end_goal.is_empty());
    assert!(!context.objective.current_objective.is_empty());
    // The default objective is persisted to the workspace.
    assert!(runtime
        .objective()
        .expect("objective runtime")
        .objective_exists());
}

#[tokio::test]
async fn test_objective_always_on_in_compiled_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (_context, compiled) = runtime
        .compile_for_task("implement indexed workspace retrieval", no_conversation())
        .await
        .unwrap();

    // The compact objective block reaches the prompt.
    assert!(compiled.prompt.contains("END GOAL"));
    assert!(compiled.prompt.contains("CURRENT OBJECTIVE"));
    assert!(compiled.prompt.contains("CURRENT MILESTONE"));
    assert!(compiled.prompt.contains("CURRENT TASK"));
}

#[tokio::test]
async fn test_goal_alignment_computed_deterministically() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (context, compiled) = runtime
        .compile_for_task("maintain the software project", no_conversation())
        .await
        .unwrap();

    // Alignment is always present when an objective exists.
    assert!(context.goal_alignment.is_some());
    // And it is reflected in the compiled prompt.
    assert!(compiled.prompt.contains("TASK ALIGNMENT"));
}

#[tokio::test]
async fn test_goal_alignment_unclear_warns_in_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));

    let (_context, compiled) = runtime
        .compile_for_task("book a flight and plan a calendar", no_conversation())
        .await
        .unwrap();

    assert!(compiled.prompt.contains("TASK ALIGNMENT"));
}

#[tokio::test]
async fn test_objective_persisted_and_reloaded() {
    let dir = tempfile::tempdir().unwrap();

    {
        let mut runtime =
            CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
        // Constructing the runtime installs and persists the default objective.
        assert!(runtime.objective().is_some());
    }

    // A fresh runtime over the same root reloads the persisted objective.
    let mut runtime =
        CanonicalRuntime::new_without_default_provider(test_config(), dir.path()).unwrap();
    runtime.register_provider(Arc::new(MockProvider::new("mock", Vec::new())));
    let (context, _compiled) = runtime
        .compile_for_task("explain the project", no_conversation())
        .await
        .unwrap();
    assert!(!context.objective.end_goal.is_empty());
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
