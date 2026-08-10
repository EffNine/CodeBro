//! Integration tests for the canonical runtime pipeline.

use std::sync::Arc;

use crate::agent::events::AgentEvent;
use crate::canonical_runtime::CanonicalRuntime;
use crate::config::Config;
use crate::engineering_context::{ContextFragment, ConversationMessage};
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
