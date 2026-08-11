//! Diagnostics for the canonical task runtime.
//!
//! These capture the per-task observability answers required by the runtime
//! contract: which project, which task, which intent, how much context, which
//! memories, which template, which provider, why that provider, did the
//! circuit breaker allow it, and how long each stage took.
//!
//! Diagnostics are collected observationally and surfaced on demand (verbose
//! / debug modes). They never influence task execution.

/// Observational diagnostics for one autonomous Research session (Sprint 30C).
#[derive(Debug, Clone, Default)]
pub struct ResearchDiagnostics {
    /// Whether research produced a usable result.
    pub completed: bool,
    /// Why research terminated (completed / iteration_limit / tool_limit /
    /// model_limit / timeout / cancelled / error).
    pub termination: String,
    /// Number of reasoning iterations.
    pub iterations: usize,
    /// Number of real tool calls executed.
    pub tool_calls: usize,
    /// Number of model (provider) calls.
    pub model_calls: usize,
    /// Number of files inspected.
    pub files_inspected: usize,
    /// Number of symbols surfaced.
    pub symbols_found: usize,
    /// Research duration in milliseconds.
    pub duration_ms: u64,
    /// Provider that executed the research.
    pub provider: String,
    /// Error message when research failed.
    pub error: Option<String>,
}

/// Per-task runtime diagnostics.
#[derive(Debug, Clone, Default)]
pub struct TaskDiagnostics {
    /// The user's task string.
    pub task: String,
    /// The project identity name.
    pub project: String,
    /// The workspace root.
    pub project_root: String,
    /// Intent type string mapped from context assembly.
    pub intent: String,
    /// Number of engineering memory entries injected.
    pub memory_entries: usize,
    /// Number of assembled context fragments.
    pub context_fragments: usize,
    /// Estimated prompt token count after compilation.
    pub prompt_tokens: usize,
    /// Prompt template selected by the compiler.
    pub template: String,
    /// Selected provider id.
    pub provider: String,
    /// Why the provider was selected (router reasons).
    pub routing_reason: String,
    /// Routing strategy used.
    pub strategy: String,
    /// Circuit breaker state at selection time.
    pub breaker_state: String,
    /// Whether the circuit breaker allowed the request.
    pub breaker_allowed: bool,
    /// Duration of project identity snapshot.
    pub identity_load_ms: u64,
    /// Duration of engineering memory resolution.
    pub memory_resolution_ms: u64,
    /// Duration of context assembly (tools + coordinator + assembler).
    pub assembly_ms: u64,
    /// Duration of prompt compilation.
    pub compile_ms: u64,
    /// Duration of provider routing.
    pub routing_ms: u64,
    /// Duration of provider execution (including retries).
    pub provider_execution_ms: u64,
    /// Total task wall time.
    pub total_ms: u64,
    /// Explicit verification results (build / tests), when run.
    pub verification: Option<super::VerificationSummary>,
    /// Autonomous research diagnostics (Sprint 30C), when research ran.
    pub research: Option<ResearchDiagnostics>,
}

impl TaskDiagnostics {
    /// Start a fresh diagnostics record for a task.
    pub fn new(task: impl Into<String>) -> Self {
        TaskDiagnostics {
            task: task.into(),
            breaker_allowed: true,
            ..TaskDiagnostics::default()
        }
    }

    /// Compact one-line summary for progressive-disclosure logging.
    pub fn summary_line(&self) -> String {
        format!(
            "[runtime] project={} intent={} fragments={} memory={} prompt={}tok template={} provider={} breaker={}{} | identity={}ms memory={}ms assembly={}ms compile={}ms routing={}ms exec={}ms total={}ms",
            if self.project.is_empty() { "-" } else { &self.project },
            if self.intent.is_empty() { "-" } else { &self.intent },
            self.context_fragments,
            self.memory_entries,
            self.prompt_tokens,
            if self.template.is_empty() { "-" } else { &self.template },
            if self.provider.is_empty() { "-" } else { &self.provider },
            if self.breaker_allowed { "allowed" } else { "blocked" },
            if self.breaker_state.is_empty() { String::new() } else { format!(" ({})", self.breaker_state) },
            self.identity_load_ms,
            self.memory_resolution_ms,
            self.assembly_ms,
            self.compile_ms,
            self.routing_ms,
            self.provider_execution_ms,
            self.total_ms,
        )
    }
}

impl From<crate::research::ResearchResult> for ResearchDiagnostics {
    fn from(result: crate::research::ResearchResult) -> Self {
        ResearchDiagnostics {
            completed: result.termination.is_completed(),
            termination: result.termination.to_string(),
            iterations: result.iterations,
            tool_calls: result.tool_calls,
            model_calls: result.model_calls,
            files_inspected: result.files_inspected.len(),
            symbols_found: result.symbols_found.len(),
            duration_ms: result.duration_ms,
            provider: result.provider,
            error: result.limitations.first().cloned(),
        }
    }
}
