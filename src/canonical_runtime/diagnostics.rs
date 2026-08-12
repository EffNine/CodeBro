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
    /// Whether the final prose synthesis was produced. `false` means the
    /// session ended before a final report could be written; the structured
    /// evidence trail is still preserved.
    pub synthesis_complete: bool,
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
    /// Autonomous testing diagnostics (Sprint 30D), when testing ran.
    pub testing: Option<TestingDiagnostics>,
    /// Autonomous planning diagnostics (Sprint 30E), when planning ran.
    pub planning: Option<PlanningDiagnostics>,
    /// Autonomous coding diagnostics (Sprint 30F), when coding ran.
    pub coding: Option<CodingDiagnostics>,
    /// Autonomous review diagnostics (Sprint 30G), when review ran.
    pub review: Option<ReviewDiagnostics>,
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
            synthesis_complete: result.synthesis_complete,
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

/// Observational diagnostics for one autonomous Testing session (Sprint 30D).
#[derive(Debug, Clone, Default)]
pub struct TestingDiagnostics {
    /// Whether testing produced a usable result.
    pub completed: bool,
    /// Whether the final prose synthesis was produced.
    pub synthesis_complete: bool,
    /// Why testing terminated (completed / iteration_limit / tool_limit /
    /// model_limit / timeout / cancelled / error).
    pub termination: String,
    /// Number of reasoning iterations.
    pub iterations: usize,
    /// Number of real tool calls executed.
    pub tool_calls: usize,
    /// Number of model (provider) calls.
    pub model_calls: usize,
    /// Number of validation commands executed.
    pub commands_run: usize,
    /// Number of validation failures.
    pub failures: usize,
    /// Number of files inspected.
    pub files_inspected: usize,
    /// Testing duration in milliseconds.
    pub duration_ms: u64,
    /// Provider that executed the testing.
    pub provider: String,
    /// Whether the git tracked tree was left unchanged after testing.
    pub git_tree_unchanged: bool,
    /// Error message when testing failed.
    pub error: Option<String>,
}

impl From<crate::testing::TestingResult> for TestingDiagnostics {
    fn from(result: crate::testing::TestingResult) -> Self {
        let git_tree_unchanged = result.git_tree_unchanged();
        TestingDiagnostics {
            completed: result.termination.is_completed(),
            synthesis_complete: result.synthesis_complete,
            termination: result.termination.to_string(),
            iterations: result.iterations,
            tool_calls: result.tool_calls,
            model_calls: result.model_calls,
            commands_run: result.commands_run.len(),
            failures: result.failures.len(),
            files_inspected: result.files_inspected.len(),
            duration_ms: result.duration_ms,
            provider: result.provider,
            git_tree_unchanged,
            error: result.limitations.first().cloned(),
        }
    }
}

/// Observational diagnostics for one autonomous Planning session (Sprint 30E).
#[derive(Debug, Clone, Default)]
pub struct PlanningDiagnostics {
    /// Whether planning produced a usable result.
    pub completed: bool,
    /// Whether the final implementation plan was produced.
    pub synthesis_complete: bool,
    /// Why planning terminated (completed / iteration_limit / tool_limit /
    /// model_limit / timeout / cancelled / error).
    pub termination: String,
    /// Number of reasoning iterations.
    pub iterations: usize,
    /// Number of read-only tool calls executed.
    pub tool_calls: usize,
    /// Number of model (provider) calls.
    pub model_calls: usize,
    /// Number of concrete plan steps extracted.
    pub plan_steps: usize,
    /// Number of affected files named by the plan.
    pub affected_files: usize,
    /// Number of risks surfaced by the plan.
    pub risks: usize,
    /// Planning duration in milliseconds.
    pub duration_ms: u64,
    /// Provider that executed the planning.
    pub provider: String,
    /// Error message when planning failed.
    pub error: Option<String>,
}

impl From<crate::planning::PlanningResult> for PlanningDiagnostics {
    fn from(result: crate::planning::PlanningResult) -> Self {
        PlanningDiagnostics {
            completed: result.termination.is_completed(),
            synthesis_complete: result.synthesis_complete,
            termination: result.termination.to_string(),
            iterations: result.iterations,
            tool_calls: result.tool_calls,
            model_calls: result.model_calls,
            plan_steps: result.plan.len(),
            affected_files: result.affected_files.len(),
            risks: result.risks.len(),
            duration_ms: result.duration_ms,
            provider: result.provider,
            error: result.limitations.first().cloned(),
        }
    }
}

/// Observational diagnostics for one autonomous Coding session (Sprint 30F).
#[derive(Debug, Clone, Default)]
pub struct CodingDiagnostics {
    /// Whether coding reached a terminal state (any non-error termination).
    pub completed: bool,
    /// Whether the final implementation summary was produced.
    pub synthesis_complete: bool,
    /// Why coding terminated (completed / iteration_limit / tool_limit /
    /// model_limit / timeout / cancelled / error).
    pub termination: String,
    /// Number of reasoning iterations.
    pub iterations: usize,
    /// Number of tool calls executed (including verification commands).
    pub tool_calls: usize,
    /// Number of model (provider) calls.
    pub model_calls: usize,
    /// Number of changes applied and kept (not rolled back).
    pub changes_applied: usize,
    /// Number of changes created then rolled back.
    pub changes_rolled_back: usize,
    /// Number of changes that were outside the plan's file list.
    pub unplanned_changes: usize,
    /// Number of policy-checked verification commands executed.
    pub verifications_run: usize,
    /// Number of verification commands that failed.
    pub verifications_failed: usize,
    /// Number of revision attempts after failed verification.
    pub revisions: usize,
    /// Coding duration in milliseconds.
    pub duration_ms: u64,
    /// Provider that executed the coding.
    pub provider: String,
    /// Error message when coding failed.
    pub error: Option<String>,
}

impl From<crate::coding::CodingResult> for CodingDiagnostics {
    fn from(result: crate::coding::CodingResult) -> Self {
        CodingDiagnostics {
            completed: result.termination.is_completed(),
            synthesis_complete: result.synthesis_complete,
            termination: result.termination.to_string(),
            iterations: result.iterations,
            tool_calls: result.tool_calls,
            model_calls: result.model_calls,
            changes_applied: result.changes.len(),
            changes_rolled_back: result.changes.iter().filter(|c| c.rolled_back).count(),
            unplanned_changes: result.unplanned_changes.len(),
            verifications_run: result.verification.len(),
            verifications_failed: result
                .verification
                .iter()
                .filter(|r| !r.success && !r.denied)
                .count(),
            revisions: result.revisions,
            duration_ms: result.duration_ms,
            provider: result.provider,
            error: result.limitations.first().cloned(),
        }
    }
}

/// Observational diagnostics for one autonomous Review session (Sprint 30G).
#[derive(Debug, Clone, Default)]
pub struct ReviewDiagnostics {
    pub completed: bool,
    pub synthesis_complete: bool,
    pub termination: String,
    /// The review verdict: PASS / PASS_WITH_RISKS / FAIL.
    pub verdict: String,
    pub iterations: usize,
    pub tool_calls: usize,
    pub model_calls: usize,
    pub findings: usize,
    pub files_inspected: usize,
    pub changed_files: usize,
    pub unverified_changes: usize,
    pub plan_deviations: usize,
    pub duration_ms: u64,
    pub provider: String,
    pub error: Option<String>,
}

impl From<crate::review::ReviewResult> for ReviewDiagnostics {
    fn from(result: crate::review::ReviewResult) -> Self {
        ReviewDiagnostics {
            completed: result.termination.is_completed(),
            synthesis_complete: result.synthesis_complete,
            termination: result.termination.to_string(),
            verdict: result.verdict.to_string(),
            iterations: result.iterations,
            tool_calls: result.tool_calls,
            model_calls: result.model_calls,
            findings: result.findings.len(),
            files_inspected: result.reviewed_files.len(),
            changed_files: result.changed_files.len(),
            unverified_changes: result.unverified_changes.len(),
            plan_deviations: result.plan_deviations.len(),
            duration_ms: result.duration_ms,
            provider: result.provider,
            error: result.limitations.first().cloned(),
        }
    }
}
