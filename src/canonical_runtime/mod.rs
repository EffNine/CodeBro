//! Canonical Runtime — the production orchestration path.
//!
//! This module wires the canonical Sprint 20–25 subsystems into the actual
//! execution path. It is an orchestrator, not a new subsystem: every stage
//! delegates to an existing canonical component.
//!
//! ```text
//! User Request
//!      ↓
//! CanonicalRuntime (task lifecycle + orchestration)
//!      ↓
//! ProjectIdentityRuntime        → one immutable snapshot per task
//!      ↓
//! EngineeringMemoryRuntime      → resolve_for_task (ranking + token budget)
//!      ↓
//! ContextAssembler              → intent, fragments, ranking, budget
//!      ↓
//! EngineeringContextBuilder     → the immutable handoff contract
//!      ↓
//! PromptBuilder::compile_context→ canonical prompt compilation
//!      ↓
//! IntelligentProviderRouter     → authoritative provider selection
//!      ↓
//! ProviderRuntime               → circuit breaker → health → retry
//!      ↓
//! I/O provider                  → streamed response
//!      ↓
//! TaskResult                    → TUI rendering
//! ```

mod diagnostics;
pub(crate) mod execution;
mod provider_adapter;
#[cfg(test)]
mod tests;

pub use diagnostics::TaskDiagnostics;
pub use provider_adapter::ProviderAdapter;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::time::Instant as TokioInstant;

use crate::agent::coordinator::AgentCoordinator;
use crate::agent::events::AgentEvent;
use crate::agent::grounding::GroundedContext;
use crate::agent::recovery::RecoveryEngine;
use crate::agent::status::AgentStatus;
use crate::agent::task_graph::{TaskGraph, TaskStatus};
use crate::agent::tool_parser::{self, ToolCall};
use crate::assembly::{
    AssemblyConfig, ContextAssembler, ContextAssemblyRequest, ContextAssemblyResult,
    ContextFragment as AssemblyFragment, ContextPriority, ContextSource, IntentType,
};
use crate::cancellation::CancellationToken;
use crate::config::Config;
use crate::dispatcher::ToolRegistry;
use crate::engineering_context::constraints::{ConstraintCategory, EngineeringConstraint};
use crate::engineering_context::workspace::WorkspaceFile;
use crate::engineering_context::{
    ConstraintContext, ContextFragment, ConversationMessage, EngineeringContext,
    EngineeringContextBuilder, EngineeringMemoryContext, IntentPlan, ProjectIdentity,
    RuntimeContext, WorkspaceContext,
};
use crate::engineering_memory::EngineeringMemoryRuntime;
use crate::engineering_objective::{
    EngineeringObjective, EngineeringObjectiveRuntime, GoalAlignment, LazyExecutionPolicy,
};
use crate::intelligence::CodeIndexer;
use crate::project_identity::{
    LoadError, ProjectIdentityRuntime, RuntimeError as IdentityRuntimeError, StorageError,
};
use crate::prompt_builder::{CompiledPrompt, PromptBuilder};
use crate::provider_runtime::routing::ProviderRoutingDecision;
use crate::provider_runtime::{
    Capability, CircuitBreakerState, CostTracker, HealthManager, IntelligentProviderRouter,
    ProviderId, ProviderRegistry, ProviderRuntime, RetryPolicy, RouteRequest,
};
use crate::providers::{OpenAiProvider, StructuredToolCall, ToolDefinition};
use crate::scanner::ProjectInfo;
use crate::tools::shell::redact_secrets_public;
use crate::tools::{detect_workspace_root, is_toolable, run_tool_pipeline};
use crate::workspace_runtime::{LocalFileSystem, WorkspaceRuntime};
use futures::StreamExt;

/// The number of ReAct reasoning iterations before giving up.
pub(crate) const MAX_REACT_ITERATIONS: usize = 5;

/// Maximum tool calls allowed per reasoning iteration.
const MAX_TOOL_CALLS_PER_ITERATION: usize = 20;

/// Maximum total tool calls across the entire task.
const MAX_TOTAL_TOOL_CALLS: usize = 100;

/// Repeated-action threshold: if the same deterministic action fingerprint
/// appears this many times in a row, the loop terminates.
const MAX_REPEATED_ACTIONS: usize = 3;

/// Default task timeout (30 seconds). Exceeding this terminates the task.
/// This is a hard safety deadline that applies across all phases
/// (planning, model reasoning, tool execution, verification, revision).
const DEFAULT_TASK_TIMEOUT_MS: u64 = 30_000;

/// Maximum total model calls (provider invocations) across the entire task,
/// including verification revisions. Each revision re-enters the ReAct loop
/// (up to `MAX_REACT_ITERATIONS` per entry). Total bound:
///   MAX_MODEL_CALLS = MAX_REACT_ITERATIONS * (1 + MAX_VERIFICATION_REVISIONS)
///   = 5 * (1 + 2) = 15 provider calls worst-case.
const MAX_MODEL_CALLS: usize =
    MAX_REACT_ITERATIONS * (1 + 2/* default max_verification_revisions */);

/// A request to execute one engineering task.
pub struct TaskRequest<'a> {
    /// The user's task.
    pub task: &'a str,
    /// Conversation history to include in the engineering context.
    pub conversation: Vec<ConversationMessage>,
    /// Agent lifecycle events sink (TUI dashboard).
    pub emit: &'a (dyn Fn(AgentEvent) + Send + Sync),
    /// Streaming response chunks sink (TUI renderer).
    pub on_chunk: &'a (dyn Fn(&str) + Send + Sync),
}

/// Optional task-run controls. Kept separate from [`TaskRequest`] so existing
/// callers keep compiling; the TUI passes cancellation and live-PTY routing.
#[derive(Default, Clone)]
pub struct TaskOptions {
    /// Cooperative cancellation (Ctrl+C). When set, execution stops promptly
    /// and PTY-backed processes receive SIGINT.
    pub cancel: Option<CancellationToken>,
    /// Live PTY output sink: `(console_id, content)`. When set, streaming tool
    /// output and verification output are forwarded here in addition to the
    /// event stream.
    pub on_pty: Option<Arc<dyn Fn(&str, &str) + Send + Sync>>,
    /// Maximum tool calls allowed per reasoning iteration. Defaults to
    /// `MAX_TOOL_CALLS_PER_ITERATION`.
    pub max_tool_calls_per_iteration: Option<usize>,
    /// Total task timeout in milliseconds. Defaults to
    /// `DEFAULT_TASK_TIMEOUT_MS`. Zero means no timeout.
    pub task_timeout_ms: Option<u64>,
    /// Maximum number of verification revision attempts after a failure.
    /// Defaults to 2.
    pub max_verification_revisions: Option<usize>,
    /// Hard deadline for the entire task. Set at the start of
    /// `run_task_with_options`. The provider stream uses this deadline
    /// with `tokio::select!` so that streaming is interruptible.
    pub deadline: Option<TokioInstant>,
    /// When true, the coordinator runs the autonomous Research subagent
    /// (Sprint 30C) before the main execution loop and injects the
    /// evidence-backed result into the main LLM context. Defaults to false.
    pub research_enabled: bool,
}

/// The outcome of a directly-invoked streaming tool run (shell commands,
/// build/test/playwright). Carries the authoritative exit code.
#[derive(Debug, Clone)]
pub struct CommandOutcome {
    pub success: bool,
    pub exit_code: i32,
    pub output: String,
    pub cancelled: bool,
}

impl CommandOutcome {
    fn from_err(e: String) -> Self {
        CommandOutcome {
            success: false,
            exit_code: -1,
            output: e,
            cancelled: false,
        }
    }
}

/// Result of the explicit verification phase (build / tests).
#[derive(Debug, Clone, Default)]
pub struct VerificationSummary {
    pub steps: Vec<VerificationStep>,
}

#[derive(Debug, Clone)]
pub struct VerificationStep {
    /// Human label, e.g. "build" or "tests".
    pub label: String,
    /// The command that was run.
    pub command: String,
    pub success: bool,
    pub exit_code: i32,
    /// Tail of the command output, for diagnostics.
    pub output_tail: String,
}

/// The outcome of one task execution.
#[derive(Debug, Clone)]
pub struct TaskResult {
    /// Whether the task succeeded.
    pub success: bool,
    /// The final assistant response.
    pub response: String,
    /// The error message when `success` is false.
    pub error: Option<String>,
    /// Per-task runtime diagnostics.
    pub diagnostics: TaskDiagnostics,
    /// Whether the task was cancelled by the user.
    pub cancelled: bool,
}

impl TaskResult {
    /// Returns `true` if the task reached a terminal state (completed, failed,
    /// or cancelled).
    pub fn is_terminal(&self) -> bool {
        !self.response.is_empty() || self.error.is_some() || self.cancelled
    }
}

/// The canonical task / agent runtime.
///
/// Owns the lifecycle of one engineering task and orchestrates the canonical
/// subsystems. One runtime instance is constructed per task execution so that
/// project identity is snapshotted once and engineering memory is resolved
/// once per task.
pub struct CanonicalRuntime {
    config: Config,
    workspace_root: std::path::PathBuf,
    identity: ProjectIdentityRuntime,
    objective: Option<EngineeringObjectiveRuntime>,
    memory: Option<EngineeringMemoryRuntime<ProjectIdentityRuntime>>,
    assembler: ContextAssembler,
    prompt_builder: PromptBuilder,
    provider_runtime: ProviderRuntime,
    router: IntelligentProviderRouter,
    registry: ProviderRegistry,
    io_providers: HashMap<ProviderId, Arc<dyn crate::providers::Provider>>,
    tool_registry: ToolRegistry,
}

/// Trace of the authoritative routing decision for diagnostics.
#[derive(Default)]
struct RouteTrace {
    provider: String,
    reason: String,
    strategy: String,
    breaker_state: String,
    breaker_allowed: bool,
    routing_ms: u64,
    exec_ms: u64,
}

impl CanonicalRuntime {
    /// Construct a runtime for the detected workspace root.
    pub fn new(config: Config) -> std::result::Result<Self, anyhow::Error> {
        Self::new_with_root(config, detect_workspace_root())
    }

    /// Construct a runtime for an explicit workspace root.
    ///
    /// Loads (or creates) the project identity and engineering memory for the
    /// root, builds the provider runtime and intelligent router over shared
    /// registry/health/cost state, and registers the configured I/O provider.
    pub fn new_with_root(
        config: Config,
        workspace_root: impl AsRef<Path>,
    ) -> std::result::Result<Self, anyhow::Error> {
        let mut runtime = Self::new_from_parts(config, workspace_root)?;
        runtime.register_provider(Arc::new(OpenAiProvider::new(runtime.config.clone())));
        Ok(runtime)
    }

    /// Construct a runtime without registering any provider.
    ///
    /// Callers are responsible for registering providers via
    /// [`CanonicalRuntime::register_provider`]. This is the test/embedded
    /// seam; the production path uses [`CanonicalRuntime::new_with_root`],
    /// which registers the configured default provider.
    pub fn new_without_default_provider(
        config: Config,
        workspace_root: impl AsRef<Path>,
    ) -> std::result::Result<Self, anyhow::Error> {
        Self::new_from_parts(config, workspace_root)
    }

    fn new_from_parts(
        config: Config,
        workspace_root: impl AsRef<Path>,
    ) -> std::result::Result<Self, anyhow::Error> {
        let workspace_root = workspace_root.as_ref().to_path_buf();

        // Project identity: load existing or create a minimal one.
        let mut identity = ProjectIdentityRuntime::new(&workspace_root);
        if let Err(IdentityRuntimeError::Load(LoadError::Storage(StorageError::NotFound(_)))) =
            identity.load()
        {
            let project = ProjectInfo::detect(workspace_root.clone()).unwrap_or_default();
            let name = if !project.name.trim().is_empty() {
                project.name.clone()
            } else {
                workspace_root
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            };
            let language = if !project.language.trim().is_empty() {
                project.language.clone()
            } else {
                "unknown".to_string()
            };
            let _ = identity.create_minimal(name, language);
        }

        // Engineering memory over the identity provider.
        let memory = build_memory(&workspace_root, identity.clone());

        // Engineering objective: load the persisted goal hierarchy or install
        // the documented default derived from the repository docs.
        let objective = build_objective(&workspace_root);

        // Provider runtime + intelligent router over shared state so routing,
        // health and cost accounting stay coherent.
        let registry = ProviderRegistry::new();
        let health = HealthManager::new();
        let cost = CostTracker::new();
        let provider_runtime =
            ProviderRuntime::from_parts(registry.clone(), health.clone(), cost.clone());
        let router = IntelligentProviderRouter::new(registry.clone(), health.clone(), cost.clone());

        let tool_registry = build_tool_registry(&workspace_root);

        Ok(CanonicalRuntime {
            config,
            workspace_root,
            identity,
            objective,
            memory,
            assembler: ContextAssembler::new(AssemblyConfig::default()),
            prompt_builder: PromptBuilder::new(),
            provider_runtime,
            router,
            registry,
            io_providers: HashMap::new(),
            tool_registry,
        })
    }

    /// Register an I/O provider so it can be routed and executed.
    pub fn register_provider(&mut self, provider: Arc<dyn crate::providers::Provider>) {
        let adapter = ProviderAdapter::new(provider.clone());
        let id = adapter.provider_id().clone();
        let _ = self.registry.register(&adapter);
        self.provider_runtime.circuit_breakers().get_or_create(&id);
        self.io_providers.insert(id, provider);
    }

    /// Override the retry policy used for provider execution.
    pub fn with_retry_policy(&mut self, policy: RetryPolicy) {
        self.provider_runtime.set_retry_policy(policy);
    }

    /// The provider runtime (breaker / health / retry / cost / diagnostics).
    pub fn provider_runtime(&self) -> &ProviderRuntime {
        &self.provider_runtime
    }

    /// The intelligent router (authoritative provider selection).
    pub fn router(&self) -> &IntelligentProviderRouter {
        &self.router
    }

    /// The project identity runtime.
    pub fn identity(&self) -> &ProjectIdentityRuntime {
        &self.identity
    }

    /// The engineering objective runtime.
    pub fn objective(&self) -> Option<&EngineeringObjectiveRuntime> {
        self.objective.as_ref()
    }

    /// The workspace root.
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// The configured model.
    pub fn model(&self) -> &str {
        &self.config.model
    }

    // =====================================================================
    // Execution
    // =====================================================================

    /// Execute one engineering task through the canonical pipeline.
    pub async fn run_task(&mut self, req: &TaskRequest<'_>) -> TaskResult {
        self.run_task_with_options(req, TaskOptions::default())
            .await
    }

    /// Execute one engineering task through the canonical pipeline with
    /// optional cancellation and live-PTY routing.
    pub async fn run_task_with_options(
        &mut self,
        req: &TaskRequest<'_>,
        opts: TaskOptions,
    ) -> TaskResult {
        let started = Instant::now();
        let mut diag = TaskDiagnostics::new(req.task);

        // Establish a hard task-level deadline. This deadline is shared across
        // all phases (planning, model reasoning, tool execution, verification,
        // revision) and is propagated into `stream_once` so that provider
        // streaming is interruptible via `tokio::select!`.
        let deadline = opts
            .task_timeout_ms
            .filter(|t| *t > 0)
            .map(|t| TokioInstant::now() + std::time::Duration::from_millis(t));

        let opts = TaskOptions {
            deadline,
            ..opts.clone()
        };

        if opts
            .cancel
            .as_ref()
            .map(|c| c.is_cancelled())
            .unwrap_or(false)
        {
            return self.cancel(req, None, req.task, diag, started);
        }
        // Task lifecycle: begin.
        let mut graph = TaskGraph::new(req.task);
        let root_id = graph.root_task.clone();
        graph.update_status(&root_id, TaskStatus::Running);
        (req.emit)(AgentEvent::TaskGraphUpdated {
            graph: graph.clone(),
        });
        (req.emit)(AgentEvent::AgentStarted {
            agent: "main".to_string(),
            task: req.task.to_string(),
        });
        (req.emit)(AgentEvent::AgentStatusChanged {
            agent: "main".to_string(),
            status: AgentStatus::Thinking,
        });

        // Project identity snapshot (one per task).
        let identity = {
            let t = Instant::now();
            let snap = if self.identity.is_loaded() {
                self.identity.snapshot()
            } else {
                ProjectIdentity::new("unknown", "unknown")
            };
            diag.identity_load_ms = t.elapsed().as_millis() as u64;
            snap
        };
        diag.project = identity.name.clone();
        diag.project_root = self.workspace_root.to_string_lossy().to_string();

        // Engineering memory resolution.
        let t = Instant::now();
        let keywords = extract_keywords(req.task);
        let memory_ctx = self.resolve_memory(&keywords, &[]);
        diag.memory_resolution_ms = t.elapsed().as_millis() as u64;
        diag.memory_entries = memory_ctx.entry_count();

        // Context assembly + observe + reason.
        let t = Instant::now();
        let memory_entries: Vec<String> = memory_ctx
            .entries
            .iter()
            .map(|e| format!("{}: {}", e.key, e.value))
            .collect();
        let (assembly, report) = match self.observe(req, &memory_entries).await {
            Ok(assembled) => assembled,
            Err(e) => return self.fail(req, Some(&mut graph), &root_id, e, diag, started),
        };
        diag.assembly_ms = t.elapsed().as_millis() as u64;
        diag.intent = format!("{:?}", assembly.intent.intent).to_lowercase();

        // EngineeringContext handoff contract.
        let mut context = match self.build_context(req, &identity, memory_ctx, assembly, report) {
            Ok(ctx) => ctx,
            Err(e) => return self.fail(req, Some(&mut graph), &root_id, e, diag, started),
        };
        diag.context_fragments = context.fragment_count();

        // Autonomous Research (Sprint 30C): when enabled, run the read-only
        // Research subagent over the grounded context and inject its
        // evidence-backed result into the context that reaches the main LLM.
        // Failure is isolated: a research error never crashes the main task.
        if opts.research_enabled {
            let research = self
                .run_autonomous_research(req, &memory_entries, &opts)
                .await;
            match research {
                Ok(result) => {
                    let render = result.render();
                    diag.research = Some(
                        crate::canonical_runtime::diagnostics::ResearchDiagnostics::from(result),
                    );
                    context = extend_context(
                        context,
                        vec![ContextFragment {
                            source: "research".to_string(),
                            content: render,
                            relevance_score: 0.85,
                        }],
                    );
                    diag.context_fragments = context.fragment_count();
                }
                Err(e) => {
                    // Failure isolation: the main task continues with the
                    // existing context. The failure is observable but not
                    // fatal.
                    (req.emit)(AgentEvent::Log {
                        level: "pipeline".to_string(),
                        message: format!("Research subagent failed (continuing without it): {e}"),
                    });
                }
            }
        }

        // Canonical prompt compilation.
        let t = Instant::now();
        let compiled = self.prompt_builder.compile_context(&context);
        diag.compile_ms = t.elapsed().as_millis() as u64;
        diag.template = compiled.template_selection.template.as_str().to_string();
        diag.prompt_tokens = compiled.estimated_tokens();

        // Execute: routing → breaker → health → retry → provider.
        let (exec_result, route_trace) = self.run_execution_loop(req, context.clone(), &opts).await;
        diag.provider = route_trace.provider;
        diag.routing_reason = route_trace.reason;
        diag.strategy = route_trace.strategy;
        diag.breaker_state = route_trace.breaker_state;
        diag.breaker_allowed = route_trace.breaker_allowed;
        diag.routing_ms = route_trace.routing_ms;
        diag.provider_execution_ms = route_trace.exec_ms;

        match exec_result {
            Ok(mut response) => {
                // Verification revision loop: verify → (pass → complete |
                // fail → bounded revise → re-execute). Only tasks that intend
                // to modify or debug the project are verified; a successful
                // verification stops the task. No further work is generated
                // after success.
                let should_verify = context
                    .task
                    .as_ref()
                    .map(|t| t.intent_type == "execution" || t.intent_type == "debugging")
                    .unwrap_or(false);
                if should_verify {
                    let max_revisions = opts.max_verification_revisions.unwrap_or(2);
                    let mut revision = 0usize;
                    loop {
                        let verify_result = self.verify_task(req, &opts, &mut diag, started).await;
                        match verify_result {
                            Some((summary, verify_response)) => {
                                diag.verification = Some(summary);
                                if !verify_response.trim().is_empty() {
                                    response.push_str("\n\n");
                                    response.push_str(&verify_response);
                                }
                                // Verification passed → complete.
                                if verify_response.contains("passed") {
                                    break;
                                }
                                // Verification failed → bounded revision.
                                revision += 1;
                                if revision >= max_revisions {
                                    // Exhausted revision budget: task fails.
                                    return self.fail(
                                        req,
                                        Some(&mut graph),
                                        &root_id,
                                        format!(
                                            "Verification failed after {} revision(s)",
                                            max_revisions
                                        ),
                                        diag,
                                        started,
                                    );
                                }
                                (req.emit)(AgentEvent::Log {
                                    level: "pipeline".to_string(),
                                    message: format!(
                                        "Verification failed, revising (attempt {}/{})",
                                        revision, max_revisions
                                    ),
                                });
                                // Re-run the execution loop to revise.
                                let (rev_exec, rev_trace) =
                                    self.run_execution_loop(req, context.clone(), &opts).await;
                                diag.provider = rev_trace.provider;
                                diag.routing_reason = rev_trace.reason;
                                diag.strategy = rev_trace.strategy;
                                diag.breaker_state = rev_trace.breaker_state;
                                diag.breaker_allowed = rev_trace.breaker_allowed;
                                diag.routing_ms = rev_trace.routing_ms;
                                diag.provider_execution_ms = rev_trace.exec_ms;
                                match rev_exec {
                                    Ok(rev_response) => {
                                        response.push_str("\n\n");
                                        response.push_str(&rev_response);
                                        context = extend_context(
                                            context,
                                            vec![ContextFragment {
                                                source: "revision".to_string(),
                                                content: format!(
                                                    "Revision {} response: {}",
                                                    revision, rev_response
                                                ),
                                                relevance_score: 0.7,
                                            }],
                                        );
                                    }
                                    Err(e) => {
                                        return self.fail(
                                            req,
                                            Some(&mut graph),
                                            &root_id,
                                            e,
                                            diag,
                                            started,
                                        );
                                    }
                                }
                            }
                            None => {
                                // No verification applicable (no build system).
                                break;
                            }
                        }
                    }
                }

                graph.update_status(&root_id, TaskStatus::Completed);
                graph.set_result(&root_id, &response);
                (req.emit)(AgentEvent::TaskGraphUpdated {
                    graph: graph.clone(),
                });
                (req.emit)(AgentEvent::Log {
                    level: "pipeline".to_string(),
                    message: diag.summary_line(),
                });
                (req.emit)(AgentEvent::AgentCompleted {
                    agent: "main".to_string(),
                    duration_ms: started.elapsed().as_millis() as u64,
                });
                TaskResult {
                    success: true,
                    response,
                    error: None,
                    diagnostics: diag,
                    cancelled: false,
                }
            }
            Err(e) => {
                // Distinguish cancellation from other failures.
                if e == "Task cancelled" {
                    self.cancel(req, Some(&mut graph), &root_id, diag, started)
                } else {
                    self.fail(req, Some(&mut graph), &root_id, e, diag, started)
                }
            }
        }
    }

    /// Compile-only mode: assemble context and produce the compiled prompt
    /// without executing a provider. Used for observability and tests.
    pub async fn compile_for_task(
        &mut self,
        task: &str,
        conversation: Vec<ConversationMessage>,
    ) -> std::result::Result<(EngineeringContext, CompiledPrompt), String> {
        let noop_emit = |_: AgentEvent| {};
        let noop_chunk = |_: &str| {};
        let req = TaskRequest {
            task,
            conversation,
            emit: &noop_emit,
            on_chunk: &noop_chunk,
        };

        let identity = if self.identity.is_loaded() {
            self.identity.snapshot()
        } else {
            ProjectIdentity::new("unknown", "unknown")
        };
        let keywords = extract_keywords(task);
        let memory_ctx = self.resolve_memory(&keywords, &[]);
        let memory_entries: Vec<String> = memory_ctx
            .entries
            .iter()
            .map(|e| format!("{}: {}", e.key, e.value))
            .collect();
        let (assembly, report) = self.observe(&req, &memory_entries).await?;
        let context = self.build_context(&req, &identity, memory_ctx, assembly, report)?;
        let compiled = self.prompt_builder.compile_context(&context);
        Ok((context, compiled))
    }

    /// Compile-only mode that also runs the autonomous Research subagent and
    /// injects its result into the compiled prompt. Used by the Sprint 30C
    /// parent-integration test: proves ResearchResult → ContextFragment →
    /// compiled main prompt.
    pub async fn compile_for_task_with_research(
        &mut self,
        task: &str,
        conversation: Vec<ConversationMessage>,
    ) -> std::result::Result<(EngineeringContext, CompiledPrompt), String> {
        let noop_emit = |_: AgentEvent| {};
        let noop_chunk = |_: &str| {};
        let req = TaskRequest {
            task,
            conversation,
            emit: &noop_emit,
            on_chunk: &noop_chunk,
        };

        let identity = if self.identity.is_loaded() {
            self.identity.snapshot()
        } else {
            ProjectIdentity::new("unknown", "unknown")
        };
        let keywords = extract_keywords(task);
        let memory_ctx = self.resolve_memory(&keywords, &[]);
        let memory_entries: Vec<String> = memory_ctx
            .entries
            .iter()
            .map(|e| format!("{}: {}", e.key, e.value))
            .collect();
        let (assembly, report) = self.observe(&req, &memory_entries).await?;
        let mut context = self.build_context(&req, &identity, memory_ctx, assembly, report)?;

        // Assemble grounding once and run the autonomous research subagent.
        let tool_observations: Vec<String> = Vec::new();
        let grounded = crate::agent::grounding::GroundingAssembler::new(&self.workspace_root)
            .assemble_with_extras(task, &tool_observations, &memory_entries);
        match self
            .run_research_task(task, grounded, &noop_emit, None)
            .await
        {
            Ok(result) => {
                let render = result.render();
                context = extend_context(
                    context,
                    vec![ContextFragment {
                        source: "research".to_string(),
                        content: render,
                        relevance_score: 0.85,
                    }],
                );
            }
            Err(e) => {
                return Err(format!("Autonomous research failed: {e}"));
            }
        }

        let compiled = self.prompt_builder.compile_context(&context);
        Ok((context, compiled))
    }

    // =====================================================================
    // Autonomous Research (Sprint 30C)
    // =====================================================================

    /// Run the autonomous Research subagent over a grounded context using the
    /// runtime's shared provider state and a restricted read-only tool
    /// registry. Returns the structured, evidence-backed [`ResearchResult`].
    pub async fn run_research_task(
        &self,
        task: &str,
        grounding: GroundedContext,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
        cancel: Option<crate::cancellation::CancellationToken>,
    ) -> std::result::Result<crate::research::ResearchResult, String> {
        let tooling = build_research_tooling(&self.workspace_root);
        let mut subagent = crate::research::ResearchSubagent::new(
            self.provider_runtime.clone(),
            self.router.clone(),
            self.io_providers.clone(),
            tooling,
        );
        let request = crate::research::ResearchRequest::new(task, self.workspace_root.clone())
            .with_grounding(grounding);
        Ok(subagent.run(request, emit, cancel).await)
    }

    /// Run the autonomous research subagent inside the canonical task pipeline
    /// (used by `run_task_with_options`). Bounded by the task deadline and the
    /// research limits; failure is isolated and reported as an error result.
    async fn run_autonomous_research(
        &self,
        req: &TaskRequest<'_>,
        memory_entries: &[String],
        opts: &TaskOptions,
    ) -> std::result::Result<crate::research::ResearchResult, String> {
        let grounded = crate::agent::grounding::GroundingAssembler::new(&self.workspace_root)
            .assemble_with_extras(req.task, &[], memory_entries);
        let cancel = opts.cancel.clone();
        self.run_research_task(req.task, grounded, req.emit, cancel)
            .await
    }

    // =====================================================================
    // Pipeline stages
    // =====================================================================

    /// Observe (tools) and run canonical context assembly, then reason
    /// (coordinator) to produce an analysis report.
    async fn observe(
        &self,
        req: &TaskRequest<'_>,
        memory_entries: &[String],
    ) -> std::result::Result<(ContextAssemblyResult, String), String> {
        // Observe: ground truth via the existing tool pipeline.
        let mut tool_frags = Vec::new();
        if is_toolable(req.task) {
            (req.emit)(AgentEvent::AgentStatusChanged {
                agent: "main".to_string(),
                status: AgentStatus::Searching,
            });
            match run_tool_pipeline(req.task, &self.workspace_root) {
                Ok(pipeline) => {
                    let total = pipeline.tool_runs.len().max(1);
                    for (i, run) in pipeline.tool_runs.iter().enumerate() {
                        (req.emit)(AgentEvent::ToolStarted {
                            tool: run.name.clone(),
                            args: run.args.clone(),
                        });
                        (req.emit)(AgentEvent::ToolCompleted {
                            tool: run.name.clone(),
                            result: run.output.clone(),
                            success: run.success,
                        });
                        // Progress reflects the real share of completed runs.
                        (req.emit)(AgentEvent::AgentProgress {
                            agent: "main".to_string(),
                            progress: (i + 1) as f32 / total as f32,
                            action: format!("Executed {}", run.name),
                        });
                    }
                    if !pipeline.context.trim().is_empty() {
                        tool_frags.push(AssemblyFragment::new(
                            ContextSource::ToolResults,
                            ContextPriority::High,
                            pipeline.context.clone(),
                            0.9,
                        ));
                    }
                }
                Err(e) => {
                    (req.emit)(AgentEvent::Log {
                        level: "pipeline".to_string(),
                        message: format!("Tool pipeline error: {e}"),
                    });
                }
            }
        }

        // Canonical context assembly. The request is scoped and dropped before
        // the coordinator await so non-`Send` runtime handles (e.g. the
        // rusqlite-backed indexer) never live across a suspension point.
        let tool_observations: Vec<String> = tool_frags.iter().map(|f| f.content.clone()).collect();
        let assembly = {
            let mut request = ContextAssemblyRequest::new(req.task.to_string());

            let project = ProjectInfo::detect(self.workspace_root.clone()).unwrap_or_default();
            request = request.with_project_info(project);

            let ws = WorkspaceRuntime::new(
                self.workspace_root.clone(),
                Arc::new(LocalFileSystem::new()),
            );
            request = request.with_workspace(ws);

            request = request.with_tool_results(tool_frags);

            // Attach the canonical indexer when an index already exists.
            let index_db = self.workspace_root.join(".codebro").join("index.db");
            if index_db.exists() {
                if let Ok(idx) = CodeIndexer::new(index_db) {
                    request = request.with_indexer(idx);
                }
            }

            self.assembler
                .assemble(&request)
                .map_err(|e| format!("Context assembly failed: {e}"))?
        };

        // Reason: existing coordinator produces a grounded plan/analysis report.
        // Grounding is assembled once from the workspace/index and shared by
        // every subagent.
        (req.emit)(AgentEvent::AgentStatusChanged {
            agent: "main".to_string(),
            status: AgentStatus::Planning,
        });
        let report = {
            let mut coordinator = AgentCoordinator::new(6);
            let coord_emit = |e: AgentEvent| {
                // The runtime owns the task lifecycle graph; suppress the
                // coordinator's internal sub-agent graph updates.
                if !matches!(e, AgentEvent::TaskGraphUpdated { .. }) {
                    (req.emit)(e);
                }
            };
            let grounded = crate::agent::grounding::GroundingAssembler::new(&self.workspace_root)
                .assemble_with_extras(req.task, &tool_observations, memory_entries);
            coordinator
                .run_task_grounded(req.task, grounded, &coord_emit)
                .await
        };

        Ok((assembly, report))
    }

    /// Build the immutable `EngineeringContext` handoff contract.
    fn build_context(
        &self,
        req: &TaskRequest<'_>,
        identity: &ProjectIdentity,
        memory_ctx: EngineeringMemoryContext,
        assembly: ContextAssemblyResult,
        report: String,
    ) -> std::result::Result<EngineeringContext, String> {
        let intent_plan = IntentPlan {
            detected_goal: req.task.to_string(),
            intent_type: map_intent_type(&assembly.intent.intent).to_string(),
            confidence: 1.0,
            ambiguity: false,
            ambiguity_reason: None,
        };

        // Compact objective snapshot + deterministic goal alignment.
        let objective = self
            .objective
            .as_ref()
            .map(|o| o.snapshot())
            .unwrap_or_default();
        let goal_alignment = self.resolve_alignment(&objective, req.task);

        // Task-scoped, bounded, budgeted conversation.
        let conversation = bounded_conversation(&req.conversation, &LazyExecutionPolicy::default());

        let mut workspace_ctx =
            WorkspaceContext::new(self.workspace_root.to_string_lossy().to_string());
        for file in &identity.important_files {
            let size = std::fs::metadata(self.workspace_root.join(file))
                .map(|m| m.len())
                .unwrap_or(0);
            workspace_ctx = workspace_ctx.with_file(WorkspaceFile {
                path: file.clone(),
                language: language_hint(file),
                size_bytes: size as usize,
            });
        }
        workspace_ctx = workspace_ctx
            .with_git(self.workspace_root.join(".git").exists())
            .with_cargo_toml(self.workspace_root.join("Cargo.toml").exists())
            .with_package_json(self.workspace_root.join("package.json").exists())
            .with_readme(self.workspace_root.join("README.md").exists());

        let mut constraints = ConstraintContext::new();
        for constraint in &identity.known_constraints {
            constraints = constraints.add_constraint(EngineeringConstraint {
                description: constraint.clone(),
                category: ConstraintCategory::Architecture,
            });
        }

        let mut fragments: Vec<ContextFragment> = assembly
            .fragments
            .iter()
            .map(|f| ContextFragment {
                source: f.source.to_string(),
                content: f.content.clone(),
                relevance_score: f.relevance_score,
            })
            .collect();
        if !report.trim().is_empty() {
            fragments.push(ContextFragment {
                source: "agent_analysis".to_string(),
                content: report,
                relevance_score: 0.8,
            });
        }
        dedup_fragments(&mut fragments);

        let mut active_files = identity.important_files.clone();
        active_files.sort();
        active_files.dedup();

        let runtime_ctx = RuntimeContext::new()
            .with_provider(&self.config.provider, &self.config.model)
            .with_stream(true);

        let context = EngineeringContextBuilder::new()
            .project(identity.clone())
            .task(intent_plan)
            .objective(objective)
            .goal_alignment(goal_alignment)
            .workspace(workspace_ctx)
            .context_fragments(fragments)
            .memory(memory_ctx)
            .constraints(constraints)
            .runtime(runtime_ctx)
            .active_files(active_files)
            .user_request(req.task)
            .conversation(conversation)
            .build()
            .map_err(|e| format!("Engineering context build failed: {e}"))?;

        Ok(context)
    }

    /// Resolve engineering memory for a task, respecting the resolver's
    /// ranking and token budget. Never dumps all memory.
    fn resolve_memory(&self, keywords: &[String], tags: &[String]) -> EngineeringMemoryContext {
        match &self.memory {
            Some(memory) => memory.resolve_for_task(keywords, tags),
            None => EngineeringMemoryContext::new(),
        }
    }

    /// Deterministic goal alignment of a task against the project objective.
    ///
    /// This is awareness metadata, not enforcement. It never blocks.
    fn resolve_alignment(
        &self,
        objective: &EngineeringObjective,
        task: &str,
    ) -> Option<GoalAlignment> {
        if objective.is_empty() {
            return None;
        }
        let keywords = extract_keywords(task);
        Some(objective.align_task(&keywords))
    }

    /// Run the ReAct loop: compile → route → execute → act → repeat.
    ///
    /// Enforces the following loop guards:
    /// - cancellation token
    /// - max reasoning iterations
    /// - max tool calls per iteration
    /// - max total tool calls
    /// - task-level deadline (hard timeout from `run_task_with_options`)
    /// - total model-call budget (across all iterations and revisions)
    /// - repeated-action detection
    async fn run_execution_loop(
        &mut self,
        req: &TaskRequest<'_>,
        initial_context: EngineeringContext,
        opts: &TaskOptions,
    ) -> (std::result::Result<String, String>, RouteTrace) {
        let started = Instant::now();
        let mut trace = RouteTrace::default();
        let mut context = initial_context;
        let max_iterations = MAX_REACT_ITERATIONS;
        let max_tool_calls_per_iter = opts
            .max_tool_calls_per_iteration
            .unwrap_or(MAX_TOOL_CALLS_PER_ITERATION);
        let max_total_tool_calls = MAX_TOTAL_TOOL_CALLS;
        let deadline = opts.deadline;
        let mut total_tool_calls: usize = 0;
        let mut model_calls: usize = 0;
        let mut repeated_actions: Vec<String> = Vec::new();

        for iteration in 0..max_iterations {
            // 1. Cancellation check.
            if let Some(cancel) = &opts.cancel {
                if cancel.is_cancelled() {
                    return (Err("Task cancelled".to_string()), trace);
                }
            }

            // 2. Deadline check before starting a new iteration.
            if let Some(dl) = deadline {
                if dl <= TokioInstant::now() {
                    return (Err("Task timed out".to_string()), trace);
                }
            }

            // 3. Model-call budget check.
            if model_calls >= MAX_MODEL_CALLS {
                return (
                    Err(format!(
                        "Maximum model calls ({}) exceeded",
                        MAX_MODEL_CALLS
                    )),
                    trace,
                );
            }

            // 4. Canonical prompt compilation for the current context.
            let compiled = self.prompt_builder.compile_context(&context);

            // 5. Authoritative provider selection.
            let route_request = RouteRequest::new()
                .with_capabilities(vec![Capability::Streaming, Capability::ToolCalling]);
            let decision = match self.router.route(&route_request) {
                Ok(d) => d,
                Err(e) => return (Err(format!("Provider routing failed: {e}")), trace),
            };
            if trace.provider.is_empty() {
                let breaker = self
                    .provider_runtime
                    .circuit_breakers()
                    .get_or_create(decision.provider_id());
                trace = RouteTrace {
                    provider: decision.provider_id().as_str().to_string(),
                    reason: decision.score.reason.join(", "),
                    strategy: format!("{:?}", decision.strategy),
                    breaker_state: format!("{:?}", breaker.state()),
                    breaker_allowed: breaker.can_execute(),
                    routing_ms: 0,
                    exec_ms: 0,
                };
            }

            // 6. Build tool definitions for structured function calling when
            //    the selected provider supports it.
            let tool_defs: Vec<ToolDefinition> = {
                let io = self.io_providers.get(decision.provider_id());
                match io {
                    Some(provider) if provider.supports_function_calling() => {
                        self.tool_registry.tool_definitions()
                    }
                    _ => Vec::new(),
                }
            };

            // 7. Execute through ProviderRuntime gates. `stream_once` returns
            //    the assistant text plus any native structured tool calls.
            match self
                .stream_once(&decision, &compiled.prompt, &tool_defs, req.on_chunk, opts)
                .await
            {
                Ok((full, structured)) => {
                    model_calls += 1;
                    // Normalize structured and text-parsed tool calls into the
                    // same internal `ToolCall` representation.
                    let calls: Vec<ToolCall> = {
                        let parsed = if !structured.is_empty() {
                            structured
                                .into_iter()
                                .map(|c| ToolCall {
                                    id: c.id,
                                    name: c.name,
                                    arguments: c.arguments,
                                })
                                .collect::<Vec<_>>()
                        } else {
                            tool_parser::parse_tool_calls(&full).unwrap_or_default()
                        };
                        // Normalize structured and text-parsed tool calls into
                        // the same internal representation. Structured calls
                        // arrive wrapped in the `{"input": ...}` envelope;
                        // text-encoded calls may carry the same envelope. The
                        // unwrap is a no-op for raw argument strings, so it is
                        // safe to apply to every tool call.
                        parsed
                            .into_iter()
                            .map(|mut c| {
                                c.arguments = tool_parser::unwrap_tool_arguments(&c.arguments);
                                c
                            })
                            .collect()
                    };
                    if !calls.is_empty() {
                        // Enforce per-iteration tool call limit.
                        if total_tool_calls + calls.len() > max_total_tool_calls {
                            return (
                                Err(format!(
                                    "Maximum total tool calls ({}) exceeded",
                                    max_total_tool_calls
                                )),
                                trace,
                            );
                        }
                        if calls.len() > max_tool_calls_per_iter {
                            return (
                                Err(format!(
                                    "Maximum tool calls per iteration ({}) exceeded",
                                    max_tool_calls_per_iter
                                )),
                                trace,
                            );
                        }

                        let mut extra = Vec::new();
                        for call in &calls {
                            // Compute a deterministic fingerprint for repeated-action
                            // detection: tool name + first 80 chars of arguments.
                            let fingerprint = format!(
                                "{}:{}",
                                call.name,
                                &call.arguments[..call
                                    .arguments
                                    .char_indices()
                                    .take(80)
                                    .last()
                                    .map(|(i, _)| i)
                                    .unwrap_or(call.arguments.len())]
                            );
                            repeated_actions.push(fingerprint.clone());
                            // Trim repeated_actions to only keep the last
                            // MAX_REPEATED_ACTIONS entries.
                            if repeated_actions.len() > MAX_REPEATED_ACTIONS {
                                repeated_actions.remove(0);
                            }
                            // Detect repeated identical actions.
                            if repeated_actions.len() == MAX_REPEATED_ACTIONS
                                && repeated_actions.iter().all(|a| a == &repeated_actions[0])
                            {
                                return (
                                    Err(format!(
                                        "Repeated identical action detected: {}",
                                        repeated_actions[0]
                                    )),
                                    trace,
                                );
                            }

                            (req.emit)(AgentEvent::ToolStarted {
                                tool: call.name.clone(),
                                args: redact_secrets_public(&call.arguments),
                            });
                            let result = self.execute_tool(call, req.emit, opts).await;
                            (req.emit)(AgentEvent::ToolCompleted {
                                tool: call.name.clone(),
                                result: result.clone(),
                                success: !result.starts_with("Error:"),
                            });
                            extra.push(ContextFragment {
                                source: "tool_result".to_string(),
                                content: format!("Tool result for {}: {}", call.name, result),
                                relevance_score: 0.9,
                            });
                            total_tool_calls += 1;
                        }
                        context = extend_context(context, extra);
                        continue;
                    }
                    trace.exec_ms = started.elapsed().as_millis() as u64;
                    return (Ok(full), trace);
                }
                Err(e) => {
                    trace.exec_ms = started.elapsed().as_millis() as u64;
                    return (Err(e), trace);
                }
            }
        }

        trace.exec_ms = started.elapsed().as_millis() as u64;
        (
            Err(
                "Reached the maximum number of reasoning iterations without a final answer."
                    .to_string(),
            ),
            trace,
        )
    }

    /// Stream a response from the routed provider with circuit breaker gate,
    /// health reporting and retry policy. Never bypasses the circuit breaker.
    ///
    /// Delegates to the shared [`execution::stream_once`] primitive so the
    /// main agent and the autonomous Research subagent share the exact same
    /// breaker / health / retry / cancellation / deadline / structured-calling
    /// behaviour.
    async fn stream_once(
        &self,
        decision: &ProviderRoutingDecision,
        prompt: &str,
        tools: &[ToolDefinition],
        on_chunk: &(dyn Fn(&str) + Send + Sync),
        opts: &TaskOptions,
    ) -> std::result::Result<(String, Vec<StructuredToolCall>), String> {
        execution::stream_once(
            &self.provider_runtime,
            &self.io_providers,
            decision,
            prompt,
            tools,
            on_chunk,
            opts,
        )
        .await
    }

    /// Execute a single tool call via the registry, streaming PTY output live
    /// when the tool supports it.
    async fn execute_tool(
        &mut self,
        call: &ToolCall,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
        opts: &TaskOptions,
    ) -> String {
        let cancel = opts.cancel.clone();
        match self
            .tool_registry
            .execute_stream(&call.name, &call.arguments, cancel)
            .await
        {
            Ok(mut stream) => {
                let mut output = String::new();
                while let Some(chunk) = stream.chunks.next().await {
                    if !chunk.text.is_empty() {
                        // Route live output exactly once: through the dedicated
                        // PTY sink when provided, otherwise through the event
                        // stream.
                        match &opts.on_pty {
                            Some(on_pty) => on_pty("task", &chunk.text),
                            None => emit(AgentEvent::PtyOutput {
                                console: "task".to_string(),
                                content: chunk.text.clone(),
                            }),
                        }
                        output.push_str(&chunk.text);
                    }
                    if chunk.is_final {
                        break;
                    }
                }
                if output.trim().is_empty() {
                    "…".to_string()
                } else {
                    output
                }
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Finalize a task as failed.
    fn fail(
        &mut self,
        req: &TaskRequest<'_>,
        graph: Option<&mut TaskGraph>,
        root_id: &str,
        error: String,
        mut diag: TaskDiagnostics,
        started: Instant,
    ) -> TaskResult {
        diag.total_ms = started.elapsed().as_millis() as u64;
        if let Some(graph) = graph {
            graph.update_status(root_id, TaskStatus::Failed);
            (req.emit)(AgentEvent::TaskGraphUpdated {
                graph: graph.clone(),
            });
        }

        if let Ok(mut recovery) = RecoveryEngine::new() {
            if let Ok(plan) = recovery.handle_failure("main", req.task, &error) {
                (req.emit)(AgentEvent::Log {
                    level: "coordination".to_string(),
                    message: format!(
                        "Provider failure: {:?} -> {}",
                        plan.action, plan.suggested_agent
                    ),
                });
            }
        }

        (req.emit)(AgentEvent::AgentFailed {
            agent: "main".to_string(),
            error: error.clone(),
        });
        TaskResult {
            success: false,
            response: String::new(),
            error: Some(error),
            diagnostics: diag,
            cancelled: false,
        }
    }

    /// Finalize a task as cancelled.
    fn cancel(
        &mut self,
        req: &TaskRequest<'_>,
        graph: Option<&mut TaskGraph>,
        root_id: &str,
        mut diag: TaskDiagnostics,
        started: Instant,
    ) -> TaskResult {
        diag.total_ms = started.elapsed().as_millis() as u64;
        if let Some(graph) = graph {
            graph.update_status(root_id, TaskStatus::Cancelled);
            (req.emit)(AgentEvent::TaskGraphUpdated {
                graph: graph.clone(),
            });
        }
        (req.emit)(AgentEvent::AgentCancelled {
            agent: "main".to_string(),
        });
        TaskResult {
            success: false,
            response: String::new(),
            error: Some("Task cancelled".to_string()),
            diagnostics: diag,
            cancelled: true,
        }
    }

    // =====================================================================
    // Direct streaming commands (`!`, /build, /test, /playwright, verify)
    // =====================================================================

    /// Run a tool through the canonical tool platform, streaming PTY output to
    /// the live console and emitting authoritative lifecycle events. Used by
    /// shell commands (`!`), engineering commands (`/build`, `/test`,
    /// `/playwright`) and the verification phase. Never fakes events: each
    /// event corresponds to a real process state transition.
    pub async fn run_tool_streaming(
        &mut self,
        tool_name: &str,
        console_id: &str,
        args: &str,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
        cancel: CancellationToken,
    ) -> CommandOutcome {
        (emit)(AgentEvent::ToolStarted {
            tool: tool_name.to_string(),
            // Command args can contain credentials (e.g. `curl -H
            // "Authorization: Bearer ..."`); the emitted event is the
            // display/persistence surface and is redacted. Execution below uses
            // the raw `args`.
            args: redact_secrets_public(args),
        });

        let outcome = match self
            .tool_registry
            .execute_stream(tool_name, args, Some(cancel))
            .await
        {
            Ok(mut stream) => {
                let mut output = String::new();
                let mut exit_code = -1;
                let mut cancelled = false;
                let mut status = "unknown".to_string();

                while let Some(chunk) = stream.chunks.next().await {
                    if !chunk.text.is_empty() {
                        output.push_str(&chunk.text);
                        (emit)(AgentEvent::PtyOutput {
                            console: console_id.to_string(),
                            content: chunk.text,
                        });
                    }
                    if let Some(meta) = &chunk.metadata {
                        if let Some(code) = meta.strip_prefix("exit:") {
                            exit_code = code.parse().unwrap_or(-1);
                        } else if meta == "cancelled" {
                            cancelled = true;
                        } else if meta == "timeout" {
                            status = "timed out".to_string();
                        } else if meta == "error" {
                            status = "error".to_string();
                        }
                    }
                    if chunk.is_final {
                        break;
                    }
                }

                if status == "unknown" {
                    status = if cancelled {
                        "cancelled".to_string()
                    } else if exit_code == 0 {
                        "completed".to_string()
                    } else {
                        "failed".to_string()
                    };
                }

                (emit)(AgentEvent::PtyExited {
                    console: console_id.to_string(),
                    exit_code,
                    status: status.clone(),
                });
                (emit)(AgentEvent::ToolCompleted {
                    tool: tool_name.to_string(),
                    result: output.clone(),
                    success: exit_code == 0,
                });

                CommandOutcome {
                    success: exit_code == 0,
                    exit_code,
                    output,
                    cancelled,
                }
            }
            Err(e) => {
                (emit)(AgentEvent::ToolCompleted {
                    tool: tool_name.to_string(),
                    result: format!("Error: {}", e),
                    success: false,
                });
                CommandOutcome::from_err(format!("Error: {}", e))
            }
        };

        outcome
    }

    /// Run a shell command directly through the PTY console path.
    pub async fn run_shell(
        &mut self,
        command: &str,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
        cancel: CancellationToken,
    ) -> CommandOutcome {
        let console_id = uuid::Uuid::new_v4().to_string();
        self.run_tool_streaming("run_command", &console_id, command, emit, cancel)
            .await
    }

    /// Determine the build/test commands for the workspace, if any.
    fn verify_commands(&self) -> (Option<(String, String)>, Option<(String, String)>) {
        let root = &self.workspace_root;
        if root.join("Cargo.toml").exists() {
            (
                Some(("cargo build".to_string(), "cargo build".to_string())),
                Some(("cargo test".to_string(), "cargo test".to_string())),
            )
        } else if root.join("package.json").exists() {
            let has_build = {
                let content =
                    std::fs::read_to_string(root.join("package.json")).unwrap_or_default();
                content.contains("\"build\"")
            };
            let build = if has_build {
                Some(("npm run build".to_string(), "npm run build".to_string()))
            } else {
                Some(("tsc --noEmit".to_string(), "tsc --noEmit".to_string()))
            };
            (
                build,
                Some(("npm test".to_string(), "npm test".to_string())),
            )
        } else {
            (None, None)
        }
    }

    /// Run the explicit verification phase (build then tests) for the task.
    ///
    /// Each step runs a real command through the canonical tool path, streams
    /// to the live console, and records the authoritative exit code. The
    /// returned string is appended to the task response; `None` means no
    /// verification was applicable (e.g. no build system detected).
    async fn verify_task(
        &mut self,
        req: &TaskRequest<'_>,
        opts: &TaskOptions,
        diag: &mut TaskDiagnostics,
        started: Instant,
    ) -> Option<(VerificationSummary, String)> {
        let (build, test) = self.verify_commands();
        let (build_label, build_cmd) = build?;
        let test_pair = test;

        (req.emit)(AgentEvent::AgentStatusChanged {
            agent: "main".to_string(),
            status: AgentStatus::Testing,
        });

        let cancel = opts.cancel.clone().unwrap_or_else(CancellationToken::new);
        let mut steps = Vec::new();
        let mut lines = Vec::new();
        let mut all_passed = true;

        let console_build = uuid::Uuid::new_v4().to_string();
        let build_outcome = self
            .run_tool_streaming(
                "run_command",
                &console_build,
                &build_cmd,
                req.emit,
                cancel.clone(),
            )
            .await;
        steps.push(VerificationStep {
            label: build_label.clone(),
            command: build_cmd.clone(),
            success: build_outcome.success,
            exit_code: build_outcome.exit_code,
            output_tail: tail(&build_outcome.output, 400),
        });
        let build_ok = build_outcome.success;
        if !build_ok {
            all_passed = false;
        }

        if let Some((test_label, test_cmd)) = test_pair {
            // Only run tests when the build passed; a failing build makes the
            // test step meaningless.
            if build_ok {
                let console_test = uuid::Uuid::new_v4().to_string();
                let test_outcome = self
                    .run_tool_streaming(
                        "run_command",
                        &console_test,
                        &test_cmd,
                        req.emit,
                        cancel.clone(),
                    )
                    .await;
                steps.push(VerificationStep {
                    label: test_label.clone(),
                    command: test_cmd.clone(),
                    success: test_outcome.success,
                    exit_code: test_outcome.exit_code,
                    output_tail: tail(&test_outcome.output, 400),
                });
                if !test_outcome.success {
                    all_passed = false;
                }
            } else {
                steps.push(VerificationStep {
                    label: test_label.clone(),
                    command: test_cmd.clone(),
                    success: false,
                    exit_code: -1,
                    output_tail: "skipped: build failed".to_string(),
                });
            }
        }

        if all_passed {
            lines.push(format!(
                "Verification passed: {} ✓",
                steps
                    .iter()
                    .map(|s| s.label.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        } else {
            lines.push(format!(
                "Verification {}:",
                steps
                    .iter()
                    .any(|s| !s.success)
                    .then_some("failed")
                    .unwrap_or("passed")
            ));
            for step in &steps {
                let mark = if step.success { "✓" } else { "✗" };
                lines.push(format!(
                    "  {} {} (exit {})",
                    mark, step.label, step.exit_code
                ));
                if !step.success {
                    for line in step.output_tail.lines().take(6) {
                        lines.push(format!("    {}", line));
                    }
                }
            }
        }
        diag.total_ms = started.elapsed().as_millis() as u64;
        diag.verification = Some(VerificationSummary {
            steps: steps.clone(),
        });

        Some((VerificationSummary { steps }, lines.join("\n")))
    }
}

// =========================================================================
// Helpers
// =========================================================================

/// Build the shared tool registry for the ReAct loop.
fn build_tool_registry(workspace_root: &Path) -> ToolRegistry {
    let root = workspace_root.to_path_buf();
    ToolRegistry::new()
        .register(Arc::new(crate::tools::ListFiles))
        .register(Arc::new(crate::tools::ReadFile))
        .register(Arc::new(crate::tools::CreateFile))
        .register(Arc::new(crate::tools::EditFile))
        .register(Arc::new(
            crate::tools::RunCommand::new()
                .with_working_directory(root.to_string_lossy().to_string()),
        ))
        .register(Arc::new(crate::tools::GitStatus))
        .register(Arc::new(crate::tools::GitDiff))
        .register(Arc::new(crate::tools::PlaywrightTool::new(root)))
}

/// Build the restricted read-only tool registry for the autonomous Research
/// subagent. Only allowlisted tools are present; the same implementations the
/// main registry uses are reused (no duplication).
fn build_research_tooling(workspace_root: &Path) -> crate::research::ResearchTooling {
    crate::research::ResearchTooling::new(workspace_root)
}

/// Keep the tail of a large output string for diagnostics.
fn tail(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let tail: String = s
            .chars()
            .rev()
            .take(max_chars)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("…{}", tail)
    }
}

/// Load engineering memory for a workspace root.
fn build_memory(
    root: &Path,
    identity: ProjectIdentityRuntime,
) -> Option<EngineeringMemoryRuntime<ProjectIdentityRuntime>> {
    let mut memory = EngineeringMemoryRuntime::new(root, identity);
    let _ = memory.load();
    Some(memory)
}

/// Load the engineering objective for a workspace root.
///
/// The workspace objective is **optional**. When
/// `.codebro/engineering_objective.json` exists it is loaded; otherwise the
/// objective stays empty and unconfigured. CodeBro never invents or persists
/// an objective for a workspace (in particular it never installs its own
/// product goal into an arbitrary repository). A missing objective never
/// breaks task execution.
fn build_objective(root: &Path) -> Option<EngineeringObjectiveRuntime> {
    let mut runtime = EngineeringObjectiveRuntime::new(root);
    match runtime.load() {
        Ok(true) => Some(runtime),
        Ok(false) => {
            // No objective file: keep the runtime empty and unconfigured.
            // Do NOT install a guessed objective and do NOT persist one.
            Some(runtime)
        }
        Err(e) => {
            tracing::warn!("Engineering objective load failed: {}", e);
            // Fall back to an empty unconfigured runtime; never guess goals.
            Some(runtime)
        }
    }
}

/// Bound a conversation to the current task: recent messages first, capped
/// by message count and token budget. The purpose is *"what happened during
/// this engineering task?"*, not *"everything the user ever said."*
fn bounded_conversation(
    conversation: &[ConversationMessage],
    policy: &LazyExecutionPolicy,
) -> Vec<ConversationMessage> {
    let mut budget = policy.max_conversation_tokens;
    let mut kept: Vec<ConversationMessage> = Vec::new();

    for msg in conversation
        .iter()
        .rev()
        .take(policy.max_conversation_messages)
    {
        let tokens = (msg.content.len() + msg.role.len()) / 4;
        if tokens > budget {
            continue;
        }
        budget -= tokens;
        kept.push(msg.clone());
    }

    kept.reverse();
    kept
}

/// Map the assembly intent type to the prompt-compiler intent vocabulary.
fn map_intent_type(intent: &IntentType) -> &'static str {
    match intent {
        IntentType::Understanding => "question",
        IntentType::Modification | IntentType::Debugging => "execution",
        IntentType::ProjectKnowledge | IntentType::WorkspaceState => "help",
        IntentType::General => "unknown",
    }
}

/// Extract deterministic task keywords for memory resolution.
fn extract_keywords(task: &str) -> Vec<String> {
    task.to_lowercase()
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .map(|s| s.to_string())
        .collect()
}

/// Deduplicate fragments by the same content-aware fingerprint the builder
/// accepts. Two distinct fragments with equal length never collide.
fn dedup_fragments(fragments: &mut Vec<ContextFragment>) {
    let mut seen = std::collections::BTreeSet::new();
    fragments.retain(|f| {
        seen.insert(crate::assembly::sources::fragment_fingerprint(
            &f.source, &f.content,
        ))
    });
}

/// A coarse language hint for a file path.
fn language_hint(path: &str) -> String {
    match path.rsplit('.').next().map(|e| e.to_lowercase()).as_deref() {
        Some("rs") => "rust".to_string(),
        Some("py") => "python".to_string(),
        Some("js") | Some("jsx") | Some("ts") | Some("tsx") => "javascript".to_string(),
        Some("go") => "go".to_string(),
        Some("md") | Some("txt") => "markdown".to_string(),
        Some("toml") | Some("json") | Some("yaml") | Some("yml") => "config".to_string(),
        _ => "unknown".to_string(),
    }
}

/// Rebuild an `EngineeringContext` with additional tool-result fragments.
fn extend_context(context: EngineeringContext, extra: Vec<ContextFragment>) -> EngineeringContext {
    let mut fragments = context.context_fragments.clone();
    fragments.extend(extra);
    dedup_fragments(&mut fragments);

    EngineeringContextBuilder::new()
        .with_skip_validation()
        .project(context.project.clone())
        .task(context.task.clone().unwrap_or(IntentPlan {
            detected_goal: context.user_request.clone(),
            intent_type: "unknown".to_string(),
            confidence: 1.0,
            ambiguity: false,
            ambiguity_reason: None,
        }))
        .objective(context.objective.clone())
        .goal_alignment(context.goal_alignment)
        .workspace(context.workspace.clone())
        .context_fragments(fragments)
        .memory(context.memory.clone())
        .constraints(context.constraints.clone())
        .runtime(context.runtime.clone())
        .active_files(context.active_files.clone())
        .user_request(context.user_request.clone())
        .conversation(context.conversation.clone())
        .system_prompt(context.system_prompt.clone())
        .build()
        .unwrap_or(context)
}
