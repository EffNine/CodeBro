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
mod provider_adapter;
#[cfg(test)]
mod tests;

pub use diagnostics::TaskDiagnostics;
pub use provider_adapter::ProviderAdapter;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crate::agent::coordinator::AgentCoordinator;
use crate::agent::events::AgentEvent;
use crate::agent::recovery::RecoveryEngine;
use crate::agent::status::AgentStatus;
use crate::agent::task_graph::{TaskGraph, TaskStatus};
use crate::agent::tool_parser::{self, ToolCall};
use crate::assembly::{
    AssemblyConfig, ContextAssembler, ContextAssemblyRequest, ContextAssemblyResult,
    ContextFragment as AssemblyFragment, ContextPriority, ContextSource, IntentType,
};
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
    Priority, ProviderCost, ProviderId, ProviderRegistry, ProviderRuntime, RetryController,
    RetryPolicy, RouteRequest, TokenUsage,
};
use crate::providers::OpenAiProvider;
use crate::scanner::ProjectInfo;
use crate::tools::{detect_workspace_root, is_toolable, run_tool_pipeline};
use crate::workspace_runtime::{LocalFileSystem, WorkspaceRuntime};

/// The number of ReAct reasoning iterations before giving up.
const MAX_REACT_ITERATIONS: usize = 5;

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
            tool_registry: build_tool_registry(),
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
        let started = Instant::now();
        let mut diag = TaskDiagnostics::new(req.task);

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
        let (assembly, report) = match self.observe(req).await {
            Ok(assembled) => assembled,
            Err(e) => return self.fail(req, &mut graph, &root_id, e, diag, started),
        };
        diag.assembly_ms = t.elapsed().as_millis() as u64;
        diag.intent = format!("{:?}", assembly.intent.intent).to_lowercase();

        // EngineeringContext handoff contract.
        let context = match self.build_context(req, &identity, memory_ctx, assembly, report) {
            Ok(ctx) => ctx,
            Err(e) => return self.fail(req, &mut graph, &root_id, e, diag, started),
        };
        diag.context_fragments = context.fragment_count();

        // Canonical prompt compilation.
        let t = Instant::now();
        let compiled = self.prompt_builder.compile_context(&context);
        diag.compile_ms = t.elapsed().as_millis() as u64;
        diag.template = compiled.template_selection.template.as_str().to_string();
        diag.prompt_tokens = compiled.estimated_tokens();

        // Execute: routing → breaker → health → retry → provider.
        let (exec_result, route_trace) = self.run_execution_loop(req, context).await;
        diag.provider = route_trace.provider;
        diag.routing_reason = route_trace.reason;
        diag.strategy = route_trace.strategy;
        diag.breaker_state = route_trace.breaker_state;
        diag.breaker_allowed = route_trace.breaker_allowed;
        diag.routing_ms = route_trace.routing_ms;
        diag.provider_execution_ms = route_trace.exec_ms;
        diag.total_ms = started.elapsed().as_millis() as u64;

        match exec_result {
            Ok(response) => {
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
                }
            }
            Err(e) => self.fail(req, &mut graph, &root_id, e, diag, started),
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
        let (assembly, report) = self.observe(&req).await?;
        let context = self.build_context(&req, &identity, memory_ctx, assembly, report)?;
        let compiled = self.prompt_builder.compile_context(&context);
        Ok((context, compiled))
    }

    // =====================================================================
    // Pipeline stages
    // =====================================================================

    /// Observe (tools) and run canonical context assembly, then reason
    /// (coordinator) to produce an analysis report.
    async fn observe(
        &self,
        req: &TaskRequest<'_>,
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
                    for run in &pipeline.tool_runs {
                        (req.emit)(AgentEvent::ToolStarted {
                            tool: run.name.clone(),
                            args: run.args.clone(),
                        });
                        (req.emit)(AgentEvent::ToolCompleted {
                            tool: run.name.clone(),
                            result: run.output.clone(),
                            success: run.success,
                        });
                        (req.emit)(AgentEvent::AgentProgress {
                            agent: "main".to_string(),
                            progress: 0.5,
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

        // Reason: existing coordinator produces a plan/analysis report.
        (req.emit)(AgentEvent::AgentStatusChanged {
            agent: "main".to_string(),
            status: AgentStatus::Planning,
        });
        let report = {
            let mut coordinator = AgentCoordinator::new(6);
            let root_str = self.workspace_root.to_string_lossy().to_string();
            let coord_emit = |e: AgentEvent| {
                // The runtime owns the task lifecycle graph; suppress the
                // coordinator's internal sub-agent graph updates.
                if !matches!(e, AgentEvent::TaskGraphUpdated { .. }) {
                    (req.emit)(e);
                }
            };
            coordinator
                .run_task(req.task, Some(&root_str), &coord_emit)
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
    async fn run_execution_loop(
        &mut self,
        req: &TaskRequest<'_>,
        initial_context: EngineeringContext,
    ) -> (std::result::Result<String, String>, RouteTrace) {
        let started = Instant::now();
        let mut trace = RouteTrace::default();
        let mut context = initial_context;

        for _ in 0..MAX_REACT_ITERATIONS {
            // Canonical prompt compilation for the current context.
            let compiled = self.prompt_builder.compile_context(&context);

            // Authoritative provider selection.
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

            // Execute through ProviderRuntime gates.
            match self
                .stream_once(&decision, &compiled.prompt, req.on_chunk)
                .await
            {
                Ok(full) => {
                    if let Ok(calls) = tool_parser::parse_tool_calls(&full) {
                        if !calls.is_empty() {
                            let mut extra = Vec::new();
                            for call in &calls {
                                (req.emit)(AgentEvent::ToolStarted {
                                    tool: call.name.clone(),
                                    args: call.arguments.clone(),
                                });
                                let result = self.execute_tool(call).await;
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
                            }
                            context = extend_context(context, extra);
                            continue;
                        }
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
            Ok(
                "Reached the maximum number of reasoning iterations without a final answer."
                    .to_string(),
            ),
            trace,
        )
    }

    /// Stream a response from the routed provider with circuit breaker gate,
    /// health reporting and retry policy. Never bypasses the circuit breaker.
    async fn stream_once(
        &self,
        decision: &ProviderRoutingDecision,
        prompt: &str,
        on_chunk: &(dyn Fn(&str) + Send + Sync),
    ) -> std::result::Result<String, String> {
        let provider_id = decision.provider_id().clone();

        let breaker = self
            .provider_runtime
            .circuit_breakers()
            .get_or_create(&provider_id);
        if !breaker.can_execute() {
            self.provider_runtime.report_failure(&provider_id);
            return Err(format!(
                "Circuit breaker open for {} ({:?})",
                provider_id,
                breaker.state()
            ));
        }

        let io = self
            .io_providers
            .get(&provider_id)
            .cloned()
            .ok_or_else(|| format!("No provider handler registered for {provider_id}"))?;

        let policy = self.provider_runtime.retry_policy().clone();
        let mut retry = RetryController::new(policy);
        let mut attempt = 0usize;

        loop {
            match io.stream_response(prompt).await {
                Ok(mut rx) => {
                    let mut full = String::new();
                    while let Some(chunk) = rx.recv().await {
                        full.push_str(&chunk);
                        on_chunk(&chunk);
                    }
                    let tokens = TokenUsage::new(prompt.len() / 4, full.len() / 4);
                    self.provider_runtime.report_success(
                        &provider_id,
                        tokens,
                        ProviderCost::default(),
                    );
                    return Ok(full);
                }
                Err(e) => {
                    attempt += 1;
                    self.provider_runtime.report_failure(&provider_id);
                    match retry.next_attempt(std::time::Duration::ZERO, &provider_id) {
                        Ok(delay) => {
                            if !delay.is_zero() {
                                tokio::time::sleep(delay).await;
                            }
                        }
                        Err(_) => {
                            return Err(format!(
                                "Provider {} failed after {} attempt(s): {}",
                                provider_id, attempt, e
                            ));
                        }
                    }
                }
            }
        }
    }

    /// Execute a single tool call via the registry.
    async fn execute_tool(&mut self, call: &ToolCall) -> String {
        match self
            .tool_registry
            .execute(&call.name, &call.arguments)
            .await
        {
            Ok(result) => result,
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Finalize a task as failed.
    fn fail(
        &mut self,
        req: &TaskRequest<'_>,
        graph: &mut TaskGraph,
        root_id: &str,
        error: String,
        mut diag: TaskDiagnostics,
        started: Instant,
    ) -> TaskResult {
        diag.total_ms = started.elapsed().as_millis() as u64;
        graph.update_status(root_id, TaskStatus::Failed);
        (req.emit)(AgentEvent::TaskGraphUpdated {
            graph: graph.clone(),
        });

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
        }
    }
}

// =========================================================================
// Helpers
// =========================================================================

/// Build the shared tool registry for the ReAct loop.
fn build_tool_registry() -> ToolRegistry {
    ToolRegistry::new()
        .register(Arc::new(crate::tools::ListFiles))
        .register(Arc::new(crate::tools::ReadFile))
        .register(Arc::new(crate::tools::CreateFile))
        .register(Arc::new(crate::tools::EditFile))
        .register(Arc::new(crate::tools::RunCommand::new()))
        .register(Arc::new(crate::tools::GitStatus))
        .register(Arc::new(crate::tools::GitDiff))
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
