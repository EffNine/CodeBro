//! The autonomous Planning subagent execution loop (Sprint 30E).
//!
//! This is a real executor, not a template generator. It receives the user
//! objective plus the evidence of Research (files, symbols, findings) and
//! Testing (commands, exit codes), decides which read-only tool call would
//! verify a missing claim, executes it through the restricted registry,
//! observes the result, and iterates until it produces a bounded,
//! evidence-backed `PlanningResult` with concrete `PlanStep`s.
//!
//! ```text
//! PlanningRequest + GroundedContext + ResearchResult + TestingResult
//!      ↓
//! PlanningSubagent loop
//!      ├── route provider (IntelligentProviderRouter)
//!      ├── stream via the shared canonical primitive (execution::stream_once)
//!      ├── structured / text tool-call parsing
//!      ├── restricted READ-ONLY tool registry
//!      ├── observation → next decision
//!      └── reserved final implementation-plan synthesis
//!      ↓
//! PlanningResult
//! ```
//!
//! Planning is strictly read-only. It NEVER modifies files, NEVER executes
//! commands and NEVER mutates git state. The machine owns repository facts,
//! test results and tool observations; the Planning LLM owns reasoning,
//! prioritization and implementation strategy. The final plan separates facts
//! from assumptions.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::agent::events::AgentEvent;
use crate::agent::status::AgentStatus;
use crate::agent::tool_parser::{self, ToolCall};
use crate::cancellation::CancellationToken;
use crate::canonical_runtime::execution;
use crate::canonical_runtime::TaskOptions;
use crate::provider_runtime::routing::IntelligentProviderRouter;
use crate::provider_runtime::{Capability, ProviderId, ProviderRuntime, RouteRequest};
use crate::providers::StructuredToolCall;
use crate::research::contract::truncate_chars;
use crate::research::ToolObservation;

use super::contract::{
    PlanStep, PlanningEvidence, PlanningRequest, PlanningResult, PlanningRisk, PlanningTermination,
};
use super::permissions::PlanningTooling;

/// The bounded planning execution runtime.
pub struct PlanningSubagent {
    provider_runtime: ProviderRuntime,
    router: IntelligentProviderRouter,
    io_providers: HashMap<ProviderId, Arc<dyn crate::providers::Provider>>,
    tooling: PlanningTooling,
}

impl PlanningSubagent {
    /// Build a planning subagent over the caller's shared provider state and a
    /// restricted read-only tool registry. All components are reused from the
    /// canonical runtime — nothing is re-implemented.
    pub fn new(
        provider_runtime: ProviderRuntime,
        router: IntelligentProviderRouter,
        io_providers: HashMap<ProviderId, Arc<dyn crate::providers::Provider>>,
        tooling: PlanningTooling,
    ) -> Self {
        PlanningSubagent {
            provider_runtime,
            router,
            io_providers,
            tooling,
        }
    }

    /// Run one bounded planning session.
    pub async fn run(
        &mut self,
        request: PlanningRequest,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
        cancel: Option<CancellationToken>,
    ) -> PlanningResult {
        let started = Instant::now();
        let limits = request.limits.clone();
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(limits.timeout_ms);

        emit(AgentEvent::Log {
            level: "planning".to_string(),
            message: format!("Planning started: {}", request.task),
        });
        emit(AgentEvent::AgentStarted {
            agent: "planning".to_string(),
            task: request.task.clone(),
        });
        emit(AgentEvent::AgentStatusChanged {
            agent: "planning".to_string(),
            status: AgentStatus::Planning,
        });

        let mut state = PlanningState::new(request, limits);
        let mut total_tool_calls = 0usize;

        loop {
            // 1. Cancellation.
            if let Some(token) = &cancel {
                if token.is_cancelled() {
                    return self.finish(state, PlanningTermination::Cancelled, started, emit);
                }
            }
            // 2. Deadline.
            if tokio::time::Instant::now() >= deadline {
                return self.finish(state, PlanningTermination::Timeout, started, emit);
            }
            // 3. Model-call budget.
            if state.model_calls >= state.limits.max_model_calls {
                return self.finish(state, PlanningTermination::ModelLimit, started, emit);
            }
            // 4. Iteration budget.
            if state.iterations >= state.limits.max_iterations {
                return self.finish(state, PlanningTermination::IterationLimit, started, emit);
            }

            // 5. Determine the phase: evidence gathering vs final synthesis.
            //    The loop reserves one model call for the final implementation
            //    plan, so a model that keeps reading can never starve the plan.
            if !state.synthesis_attempted
                && state.model_calls >= state.limits.evidence_model_budget()
                && state.has_evidence()
            {
                state.synthesis_attempted = true;
                emit(AgentEvent::Log {
                    level: "planning".to_string(),
                    message: "Planning entering final synthesis phase".to_string(),
                });
            }
            let prompt = if state.synthesis_attempted {
                state.build_synthesis_prompt()
            } else {
                state.build_prompt()
            };

            // 6. Authoritative provider selection (identical to main agent).
            let route_request = RouteRequest::new()
                .with_capabilities(vec![Capability::Streaming, Capability::ToolCalling]);
            let decision = match self.router.route(&route_request) {
                Ok(decision) => decision,
                Err(e) => {
                    return self.finish_error(
                        state,
                        format!("Provider routing failed: {e}"),
                        started,
                        emit,
                    );
                }
            };
            let provider_id = decision.provider_id();
            let provider_model = decision.provider.display_name.clone();

            // 7. Tool definitions come from the restricted registry when the
            //    provider supports native function calling.
            let tool_defs: Vec<crate::providers::ToolDefinition> = {
                match self.io_providers.get(provider_id) {
                    Some(provider) if provider.supports_function_calling() => {
                        self.tooling.registry.tool_definitions()
                    }
                    _ => Vec::new(),
                }
            };

            let opts = TaskOptions {
                cancel: cancel.clone(),
                deadline: Some(deadline),
                ..TaskOptions::default()
            };

            emit(AgentEvent::Log {
                level: "planning".to_string(),
                message: format!("Planning model call {}", state.model_calls + 1),
            });

            // 8. Execute through the shared canonical primitive (breaker /
            //    health / retry / cancellation / deadline / structured calls).
            match execution::stream_once(
                &self.provider_runtime,
                &self.io_providers,
                &decision,
                &prompt,
                &tool_defs,
                &|_| {},
                &opts,
            )
            .await
            {
                Ok((full, structured)) => {
                    state.model_calls += 1;
                    state.iterations += 1;

                    let calls: Vec<ToolCall> = {
                        let parsed = if !structured.is_empty() {
                            structured
                                .into_iter()
                                .map(|c: StructuredToolCall| ToolCall {
                                    id: c.id,
                                    name: c.name,
                                    arguments: c.arguments,
                                })
                                .collect::<Vec<_>>()
                        } else {
                            tool_parser::parse_tool_calls(&full).unwrap_or_default()
                        };
                        // Unwrap the `{"input": ...}` argument envelope so the
                        // restricted registry receives the raw argument string.
                        parsed
                            .into_iter()
                            .map(|mut c| {
                                c.arguments = tool_parser::unwrap_tool_arguments(&c.arguments);
                                c
                            })
                            .collect()
                    };

                    // No tool call → the model produced its final plan. This
                    // is the synthesis-complete signal.
                    if calls.is_empty() {
                        state.final_answer = Some(full);
                        state.synthesis_complete = true;
                        let model = provider_model;
                        return self
                            .finish(state, PlanningTermination::Completed, started, emit)
                            .with_provider(provider_id.as_str().to_string(), model);
                    }

                    // The reserved synthesis call must not keep gathering
                    // evidence: terminate honestly — the structured evidence
                    // gathered so far is preserved and no plan is fabricated.
                    if state.synthesis_attempted {
                        return self.finish(state, PlanningTermination::ModelLimit, started, emit);
                    }

                    // 9. Tool-call budget.
                    if total_tool_calls + calls.len() > state.limits.max_tool_calls {
                        return self.finish(state, PlanningTermination::ToolLimit, started, emit);
                    }

                    for call in &calls {
                        total_tool_calls += 1;
                        state.tool_calls += 1;
                        emit(AgentEvent::ToolStarted {
                            tool: call.name.clone(),
                            args: crate::tools::shell::redact_secrets_public(&call.arguments),
                        });
                        emit(AgentEvent::Log {
                            level: "planning".to_string(),
                            message: format!(
                                "Planning tool call {}: {}",
                                state.tool_calls, call.name
                            ),
                        });
                        let result = self
                            .tooling
                            .execute(&call.name, &call.arguments, cancel.clone())
                            .await;
                        let truncated = truncate_chars(&result, state.limits.max_tool_result_chars);
                        let success = !result.starts_with("Error:");
                        emit(AgentEvent::Log {
                            level: "planning".to_string(),
                            message: format!(
                                "Planning tool result {}: success={}",
                                state.tool_calls, success
                            ),
                        });
                        emit(AgentEvent::ToolCompleted {
                            tool: call.name.clone(),
                            result: truncated.clone(),
                            success,
                        });
                        state.observe(
                            ToolObservation {
                                name: call.name.clone(),
                                arguments: call.arguments.clone(),
                                result: truncated,
                                success,
                            },
                            &result,
                        );
                    }
                }
                Err(e) => {
                    return self.finish_error(state, e, started, emit);
                }
            }
        }
    }

    /// Assemble the final result for a terminating session.
    fn finish(
        &self,
        state: PlanningState,
        termination: PlanningTermination,
        started: Instant,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> PlanningResult {
        if termination.is_completed() {
            emit(AgentEvent::AgentCompleted {
                agent: "planning".to_string(),
                duration_ms: started.elapsed().as_millis() as u64,
            });
        } else {
            emit(AgentEvent::Log {
                level: "planning".to_string(),
                message: format!("Planning terminated: {}", termination),
            });
        }
        state.build_result(termination, started.elapsed().as_millis() as u64)
    }

    /// Assemble an error result for a session interrupted by a failure.
    fn finish_error(
        &self,
        mut state: PlanningState,
        error: String,
        started: Instant,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> PlanningResult {
        state.error = Some(error.clone());
        emit(AgentEvent::AgentFailed {
            agent: "planning".to_string(),
            error: error.clone(),
        });
        emit(AgentEvent::Log {
            level: "planning".to_string(),
            message: format!("Planning failed: {}", error),
        });
        state.build_result(
            PlanningTermination::Error,
            started.elapsed().as_millis() as u64,
        )
    }
}

/// Accumulated planning session state.
struct PlanningState {
    request: PlanningRequest,
    limits: super::limits::PlanningLimits,
    iterations: usize,
    tool_calls: usize,
    model_calls: usize,
    /// Whether the loop has switched from evidence gathering to the reserved
    /// final implementation-plan synthesis call.
    synthesis_attempted: bool,
    /// Whether the final plan synthesis was produced.
    synthesis_complete: bool,
    observations: Vec<ToolObservation>,
    files_inspected: Vec<PathBuf>,
    final_answer: Option<String>,
    error: Option<String>,
}

impl PlanningState {
    fn new(request: PlanningRequest, limits: super::limits::PlanningLimits) -> Self {
        PlanningState {
            request,
            limits,
            iterations: 0,
            tool_calls: 0,
            model_calls: 0,
            synthesis_attempted: false,
            synthesis_complete: false,
            observations: Vec::new(),
            files_inspected: Vec::new(),
            final_answer: None,
            error: None,
        }
    }

    /// Record one real tool observation. Planning only tracks inspected files
    /// — symbol extraction is research's job; Planning consumes those symbols.
    fn observe(&mut self, observation: ToolObservation, _full_result: &str) {
        match observation.name.as_str() {
            "read_file" => {
                if let Some(path) = parse_arg_path(&observation.arguments) {
                    self.add_file(PathBuf::from(path));
                }
            }
            "list_files" => {
                for path in list_output_paths(&observation.result) {
                    self.add_file(path);
                }
            }
            _ => {}
        }
        self.observations.push(observation);
    }

    fn add_file(&mut self, path: PathBuf) {
        let path = self.relativize(path);
        if !self.files_inspected.contains(&path) {
            self.files_inspected.push(path);
        }
    }

    /// Whether the session has any evidence worth planning from: the Research
    /// / Testing input evidence counts, as does any real tool observation.
    fn has_evidence(&self) -> bool {
        if self.tool_calls > 0 {
            return true;
        }
        if let Some(research) = &self.request.research {
            if !research.files_inspected.is_empty()
                || !research.symbols_found.is_empty()
                || !research.findings.is_empty()
            {
                return true;
            }
        }
        if let Some(testing) = &self.request.testing {
            if !testing.commands_run.is_empty() || !testing.failures.is_empty() {
                return true;
            }
        }
        !self.request.grounding.relevant_files.is_empty()
    }

    /// Make an absolute inspected path relative to the workspace root so the
    /// result is stable across machines.
    fn relativize(&self, path: PathBuf) -> PathBuf {
        if path.is_absolute() {
            path.strip_prefix(&self.request.workspace_root)
                .map(|p| p.to_path_buf())
                .unwrap_or(path)
        } else {
            path
        }
    }

    // =====================================================================
    // Evidence rendering (shared between the regular and synthesis prompts)
    // =====================================================================

    fn render_grounding(&self) -> String {
        let grounding = &self.request.grounding;
        let mut out = String::new();
        out.push_str(&format!(
            "Project: {} ({})\n",
            grounding.project_name, grounding.project_language
        ));
        out.push_str(&format!(
            "Workspace root: {}\n",
            self.request.workspace_root.display()
        ));
        if !grounding.relevant_files.is_empty() {
            out.push_str(&format!(
                "Relevant files: {}\n",
                grounding.relevant_files.join(", ")
            ));
        }
        if !grounding.related_symbols.is_empty() {
            out.push_str(&format!(
                "Related symbols: {}\n",
                grounding.related_symbols.join(", ")
            ));
        }
        if !grounding.dependencies.is_empty() {
            out.push_str(&format!(
                "Dependencies: {}\n",
                grounding.dependencies.join(", ")
            ));
        }
        if !grounding.build_info.is_empty() {
            out.push_str(&format!("Build info: {}\n", grounding.build_info));
        }
        out
    }

    /// RESEARCH EVIDENCE section. The planner consumes these facts rather than
    /// rediscovering them; the section header is stable so provenance is
    /// auditable and testable.
    fn render_research(&self) -> String {
        let Some(research) = &self.request.research else {
            return "(no research evidence available)\n".to_string();
        };
        let mut out = String::new();
        if !research.files_inspected.is_empty() {
            out.push_str(&format!(
                "Files inspected: {}\n",
                research
                    .files_inspected
                    .iter()
                    .map(|f| f.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !research.symbols_found.is_empty() {
            out.push_str(&format!(
                "Symbols found: {}\n",
                research.symbols_found.join(", ")
            ));
        }
        if !research.findings.is_empty() {
            out.push_str("Findings:\n");
            for finding in &research.findings {
                out.push_str(&format!(
                    "- {}{}\n",
                    finding.statement,
                    finding
                        .file
                        .as_ref()
                        .map(|f| format!(" (file: {})", f.display()))
                        .unwrap_or_default()
                ));
                if let Some(symbol) = &finding.symbol {
                    out.push_str(&format!("  symbol: {}\n", symbol));
                }
            }
        }
        if out.is_empty() {
            out.push_str("(research produced no evidence)\n");
        }
        out
    }

    /// TESTING EVIDENCE section. The authoritative machine facts (exit codes)
    /// are what the planner may rely on; model prose is interpretation only.
    fn render_testing(&self) -> String {
        let Some(testing) = &self.request.testing else {
            return "(no testing evidence available)\n".to_string();
        };
        let mut out = String::new();
        if testing.commands_run.is_empty() {
            out.push_str("(no validation commands were executed)\n");
        } else {
            out.push_str("Command results (AUTHORITATIVE machine facts — trust the exit codes, not prose):\n");
            for command in &testing.commands_run {
                out.push_str(&format!(
                    "- {} → exit_code: {}, success: {}{}{}\n",
                    command.command,
                    command.exit_code,
                    command.success,
                    if command.denied {
                        format!(
                            ", denied: {}",
                            command.denied_reason.as_deref().unwrap_or("")
                        )
                    } else {
                        String::new()
                    },
                    if command.timeout {
                        ", timed_out: true"
                    } else {
                        ""
                    }
                ));
            }
        }
        if !testing.failures.is_empty() {
            out.push_str("Failures:\n");
            for failure in &testing.failures {
                out.push_str(&format!(
                    "- {} ({}) exit_code: {}\n",
                    failure.command,
                    failure.kind.as_str(),
                    failure.exit_code
                ));
            }
        }
        if !testing.summary.is_empty() {
            out.push_str("Testing prose summary (advisory only):\n");
            out.push_str(&truncate_chars(&testing.summary, 600));
            out.push('\n');
        }
        out
    }

    fn render_observations(&self, include_results: bool) -> String {
        if self.observations.is_empty() {
            return "(none yet)\n".to_string();
        }
        let mut out = String::new();
        for (i, observation) in self.observations.iter().enumerate() {
            if include_results {
                out.push_str(&format!(
                    "  {}. {} {} → {}\n",
                    i + 1,
                    observation.name,
                    observation.arguments,
                    observation.result
                ));
            } else {
                out.push_str(&format!(
                    "  {}. {} {}\n",
                    i + 1,
                    observation.name,
                    observation.arguments
                ));
            }
        }
        out
    }

    // =====================================================================
    // Prompts
    // =====================================================================

    /// Compile the planning prompt for the next model call. Evidence sections
    /// (GROUNDING / RESEARCH EVIDENCE / TESTING EVIDENCE / CURRENT PLANNING
    /// OBSERVATIONS / USER OBJECTIVE) stay distinct — provenance matters.
    fn build_prompt(&self) -> String {
        let mut prompt = String::new();
        prompt.push_str(
            "You are CodeBro's autonomous Planning subagent. You produce an evidence-backed IMPLEMENTATION PLAN. You are strictly READ-ONLY.\n\n",
        );
        prompt.push_str(&format!("USER OBJECTIVE:\n{}\n\n", self.request.task));

        prompt.push_str("GROUNDING (initial repository knowledge):\n");
        prompt.push_str(&self.render_grounding());
        prompt.push('\n');

        prompt.push_str("RESEARCH EVIDENCE (what actually exists — consume, do not rediscover):\n");
        prompt.push_str(&self.render_research());
        prompt.push('\n');

        prompt.push_str("TESTING EVIDENCE (authoritative validation facts):\n");
        prompt.push_str(&self.render_testing());
        prompt.push('\n');

        prompt.push_str("CURRENT PLANNING OBSERVATIONS (your own read-only tool results):\n");
        prompt.push_str(&self.render_observations(true));

        prompt.push_str("\nAVAILABLE TOOLS (read-only only — you must never modify anything):\n");
        for tool in self.request.limits.describe_tools() {
            prompt.push_str(&format!("- {}\n", tool));
        }

        prompt.push_str(&format!(
            "\nINSTRUCTIONS:\n1. Start from the RESEARCH and TESTING evidence above. Only inspect the repository when you need to VERIFY a claim that the evidence alone does not settle.\n2. Use a small number of TARGETED reads (read_file on the exact file a research finding names). Do NOT launch broad repository scans — your tool budget is {}\n3. You MUST call at least one tool before producing the final plan UNLESS the existing evidence already answers the objective.\n4. You have a bounded evidence budget: only {} evidence-gathering call(s) remain before the final synthesis. Gather only the evidence you actually need.\n5. You are planning ONLY. Never modify files, never run commands, never touch git state.\n\nPLANNING STEP {}:\n",
            self.limits.max_tool_calls,
            self.limits.evidence_model_budget().saturating_sub(self.model_calls).max(1),
            self.iterations + 1
        ));
        prompt
    }

    /// Compile the reserved final implementation-plan synthesis prompt. The
    /// model sees the full evidence trail and must produce the structured plan
    /// WITHOUT any further tool calls.
    fn build_synthesis_prompt(&self) -> String {
        let mut prompt = String::new();
        prompt.push_str(
            "You are CodeBro's autonomous Planning subagent final synthesis step. Produce the FINAL IMPLEMENTATION PLAN. No tools are available.\n\n",
        );
        prompt.push_str(&format!("USER OBJECTIVE:\n{}\n\n", self.request.task));

        prompt.push_str("GROUNDING (initial repository knowledge):\n");
        prompt.push_str(&self.render_grounding());
        prompt.push('\n');

        prompt.push_str("RESEARCH EVIDENCE (what actually exists):\n");
        prompt.push_str(&self.render_research());
        prompt.push('\n');

        prompt.push_str("TESTING EVIDENCE (authoritative validation facts):\n");
        prompt.push_str(&self.render_testing());
        prompt.push('\n');

        prompt.push_str("CURRENT PLANNING OBSERVATIONS (your read-only tool results):\n");
        prompt.push_str(&self.render_observations(true));

        prompt.push_str(
            "\nINSTRUCTIONS:\n1. Synthesize the evidence into a concise, concrete FINAL IMPLEMENTATION PLAN that answers the OBJECTIVE.\n2. Do NOT call any tools. This is the final synthesis step; the evidence budget is exhausted.\n3. Base every claim ONLY on the evidence above or the grounded context. If the evidence does not answer the objective, say so explicitly (as an Assumption) rather than inventing details.\n4. Never generate code patches, diffs or source files — this is a PLAN, not an implementation.\n5. Distinguish facts from assumptions. Every step must name real files and symbols from the evidence.\n\nFINAL IMPLEMENTATION PLAN FORMAT (follow exactly):\n## Existing implementation\n<what currently exists>\n\n## Required change\n<what must change to satisfy the objective>\n\nStep 1: <concrete action, e.g. \"Modify run_execution_loop in src/canonical_runtime/mod.rs\">\nFiles: <exact file paths, comma separated>\nSymbols: <symbol names, comma separated>\nReason: <why this change is required>\nDepends: <dependencies or coupling>\nValidate: <concrete validation command(s)>\nTests: <test file to update or add>\nRisk: <potential regression point>\n\n...repeat for every step...\n\nDependencies: <cross-cutting dependencies>\nAssumption: <anything you could not verify — never silently a fact>\nRisk: <plan-level risks>\n\nFINAL IMPLEMENTATION PLAN:\n",
        );
        prompt
    }

    // =====================================================================
    // Result assembly
    // =====================================================================

    fn build_result(&self, termination: PlanningTermination, duration_ms: u64) -> PlanningResult {
        let answer = self.final_answer.clone().unwrap_or_default();
        let summary = if answer.is_empty() {
            self.default_summary(termination)
        } else {
            answer.clone()
        };
        let plan = self.extract_plan(&answer);
        let affected_files = union_paths(&plan);
        let affected_symbols = union_symbols(&plan);
        let tests_to_update = union_tests(&plan);
        let dependencies = self.extract_dependencies(&answer, &plan);
        let risks = self.extract_risks(&answer, &plan);
        let assumptions = self.extract_assumptions(&answer);
        let evidence = self.build_evidence();
        let plan = self.attach_step_evidence(plan, &evidence);
        let limitations = self.build_limitations(termination);
        let output_size = self.estimate_output();

        PlanningResult {
            summary,
            plan,
            affected_files,
            affected_symbols,
            dependencies,
            tests_to_update,
            risks,
            assumptions,
            evidence,
            tool_calls: self.tool_calls,
            iterations: self.iterations,
            model_calls: self.model_calls,
            termination,
            synthesis_complete: self.synthesis_complete,
            tool_observations: self.observations.clone(),
            limitations,
            duration_ms,
            output_size,
            provider: String::new(),
            model: String::new(),
        }
    }

    /// A deterministic summary when the model never produced a final plan.
    fn default_summary(&self, termination: PlanningTermination) -> String {
        let evidence = if self.tool_calls == 0 {
            "consumed Research/Testing evidence".to_string()
        } else {
            format!("{} read-only observation(s)", self.tool_calls)
        };
        format!(
            "Planning terminated with status '{}' after {} iteration(s), {} model call(s), {} tool call(s) and {}.",
            termination,
            self.iterations,
            self.model_calls,
            self.tool_calls,
            evidence
        )
    }

    fn build_limitations(&self, termination: PlanningTermination) -> Vec<String> {
        let mut limitations = Vec::new();
        if let Some(error) = &self.error {
            limitations.push(error.clone());
        }
        match termination {
            PlanningTermination::Completed => {}
            PlanningTermination::IterationLimit => {
                limitations.push("iteration limit reached".to_string());
            }
            PlanningTermination::ToolLimit => {
                limitations.push("tool-call limit reached".to_string());
            }
            PlanningTermination::ModelLimit => {
                limitations.push("model-call limit reached".to_string());
            }
            PlanningTermination::Timeout => {
                limitations.push("planning timeout reached".to_string());
            }
            PlanningTermination::Cancelled => {
                limitations.push("planning cancelled".to_string());
            }
            PlanningTermination::Error => {}
        }
        limitations
    }

    fn estimate_output(&self) -> usize {
        let mut size = 0usize;
        for observation in &self.observations {
            size += observation.name.len() + observation.arguments.len() + observation.result.len();
        }
        size + self.final_answer.clone().unwrap_or_default().len()
    }

    // =====================================================================
    // Plan extraction (deterministic structured parsing of the synthesis)
    // =====================================================================

    /// Parse the model's final plan text into concrete [`PlanStep`]s. A step
    /// is a `Step N:` header or a numbered line, optionally followed by
    /// `Files:`, `Symbols:`, `Reason:`, `Depends:`, `Validate:`, `Tests:` and
    /// `Risk:` field lines. Steps with no fields degrade to a concrete action
    /// (they are still real steps, not fabrications).
    fn extract_plan(&self, answer: &str) -> Vec<PlanStep> {
        let mut steps = Vec::new();
        let mut current: Option<PlanStep> = None;

        for raw in answer.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((order, action)) = parse_step_header(line) {
                if let Some(step) = current.take() {
                    steps.push(step);
                }
                current = Some(PlanStep {
                    order,
                    action,
                    target_files: Vec::new(),
                    target_symbols: Vec::new(),
                    rationale: String::new(),
                    dependencies: Vec::new(),
                    validation: Vec::new(),
                    risk: String::new(),
                    evidence: Vec::new(),
                });
                continue;
            }
            let Some(step) = current.as_mut() else {
                continue;
            };
            if let Some(files) = strip_field(line, "Files:") {
                for path in parse_paths(files) {
                    if !step.target_files.contains(&path) {
                        step.target_files.push(path);
                    }
                }
            } else if let Some(symbols) = strip_field(line, "Symbols:") {
                for symbol in parse_tokens(symbols)
                    .into_iter()
                    .filter(|s| looks_like_identifier(s))
                {
                    if !step.target_symbols.contains(&symbol) {
                        step.target_symbols.push(symbol);
                    }
                }
            } else if let Some(reason) = strip_field(line, "Reason:") {
                step.rationale = reason.to_string();
            } else if let Some(deps) = strip_field(line, "Depends:") {
                for dep in parse_tokens(deps) {
                    if !step.dependencies.contains(&dep) {
                        step.dependencies.push(dep);
                    }
                }
            } else if let Some(validation) = strip_field(line, "Validate:") {
                for command in parse_commands(validation) {
                    if !step.validation.contains(&command) {
                        step.validation.push(command);
                    }
                }
            } else if let Some(tests) = strip_field(line, "Tests:") {
                for path in parse_paths(tests) {
                    if !step.target_files.contains(&path) {
                        step.target_files.push(path.clone());
                    }
                }
            } else if let Some(risk) = strip_field(line, "Risk:") {
                // The FIRST Risk field belongs to the step; later "Risk:"
                // lines (after the step fields, e.g. after "Dependencies:")
                // are plan-level risks and must not overwrite the step risk.
                if step.risk.is_empty() {
                    step.risk = risk.to_string();
                }
            }
        }
        if let Some(step) = current.take() {
            steps.push(step);
        }
        steps
    }

    /// Cross-cutting dependencies: standalone `Dependencies:` lines plus the
    /// union of every step's dependency fields.
    fn extract_dependencies(&self, answer: &str, plan: &[PlanStep]) -> Vec<String> {
        let mut dependencies = Vec::new();
        for line in answer.lines() {
            if let Some(rest) = strip_field(line.trim(), "Dependencies:") {
                for dep in parse_tokens(rest) {
                    if !dependencies.contains(&dep) {
                        dependencies.push(dep);
                    }
                }
            }
        }
        for step in plan {
            for dep in &step.dependencies {
                if !dependencies.contains(dep) {
                    dependencies.push(dep.clone());
                }
            }
        }
        dependencies
    }

    /// Plan-level risks: standalone `Risk:` lines and the optional trailing
    /// `(severity: x) [mitigation: y]` refinement. Step-level `Risk:` fields
    /// stay with their steps and never become plan-level risks.
    fn extract_risks(&self, answer: &str, plan: &[PlanStep]) -> Vec<PlanningRisk> {
        let mut risks: Vec<PlanningRisk> = Vec::new();
        for raw in answer.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = strip_field(line, "Risk:") {
                let mut severity = "unknown".to_string();
                let mut mitigation = String::new();
                let mut description = rest.to_string();
                // A trailing " (severity: x) [mitigation: y]" refinement.
                if let Some(idx) = description.find(" (severity:") {
                    let tail = &description[idx..];
                    if let Some(s) = tail
                        .split_once("severity:")
                        .and_then(|(_, s)| s.split([')', ']']).next().map(|s| s.trim().to_string()))
                    {
                        severity = s;
                    }
                    if let Some(m) = tail
                        .split_once("mitigation:")
                        .and_then(|(_, m)| m.split(']').next().map(|m| m.trim().to_string()))
                    {
                        mitigation = m;
                    }
                    description = description[..idx].trim().to_string();
                }
                // Step-level risks are not plan-level risks.
                if !plan.iter().any(|s| s.risk == description) {
                    risks.push(PlanningRisk {
                        description,
                        severity,
                        mitigation,
                    });
                }
            }
        }
        risks
    }

    /// Assumptions: every `Assumption:` line. The planner must never silently
    /// turn an assumption into a fact.
    fn extract_assumptions(&self, answer: &str) -> Vec<String> {
        let mut assumptions = Vec::new();
        for line in answer.lines() {
            let line = line.trim();
            if let Some(rest) = strip_field(line, "Assumption:") {
                assumptions.push(rest.to_string());
            }
        }
        assumptions
    }

    /// The full evidence-provenance trail: research facts, testing facts,
    /// grounding files and the planner's own read-only observations.
    fn build_evidence(&self) -> Vec<PlanningEvidence> {
        let mut evidence = Vec::new();

        if let Some(research) = &self.request.research {
            for file in &research.files_inspected {
                evidence.push(PlanningEvidence {
                    source: "research".to_string(),
                    reference: file.display().to_string(),
                    summary: "file inspected by research".to_string(),
                });
            }
            for symbol in &research.symbols_found {
                evidence.push(PlanningEvidence {
                    source: "research".to_string(),
                    reference: symbol.clone(),
                    summary: "symbol surfaced by research".to_string(),
                });
            }
            for finding in research.findings.iter().take(12) {
                evidence.push(PlanningEvidence {
                    source: "research".to_string(),
                    reference: finding
                        .file
                        .as_ref()
                        .map(|f| f.display().to_string())
                        .unwrap_or_default(),
                    summary: truncate_chars(&finding.statement, 200),
                });
            }
        }

        if let Some(testing) = &self.request.testing {
            for command in &testing.commands_run {
                evidence.push(PlanningEvidence {
                    source: "testing".to_string(),
                    reference: command.command.clone(),
                    summary: format!(
                        "command exit_code {} success {}",
                        command.exit_code, command.success
                    ),
                });
            }
            for failure in &testing.failures {
                evidence.push(PlanningEvidence {
                    source: "testing".to_string(),
                    reference: failure.command.clone(),
                    summary: format!(
                        "failure kind {} exit_code {}",
                        failure.kind.as_str(),
                        failure.exit_code
                    ),
                });
            }
        }

        for file in &self.request.grounding.relevant_files {
            evidence.push(PlanningEvidence {
                source: "grounding".to_string(),
                reference: file.clone(),
                summary: "file from grounded context".to_string(),
            });
        }

        for observation in &self.observations {
            let reference = if observation.name == "read_file" {
                parse_arg_path(&observation.arguments)
                    .unwrap_or_else(|| observation.arguments.clone())
            } else {
                observation.arguments.clone()
            };
            evidence.push(PlanningEvidence {
                source: "planning_read".to_string(),
                reference,
                summary: truncate_chars(&observation.result, 200),
            });
        }
        evidence
    }

    /// Link each plan step to the evidence that supports it: research
    /// files/symbols, the planner's own read observations of the target files,
    /// grounding files, and authoritative testing command facts.
    fn attach_step_evidence(
        &self,
        plan: Vec<PlanStep>,
        all_evidence: &[PlanningEvidence],
    ) -> Vec<PlanStep> {
        let mut linked = Vec::new();
        for mut step in plan {
            let mut attached: Vec<String> = Vec::new();
            let research = self.request.research.as_ref();
            let testing = self.request.testing.as_ref();

            for file in &step.target_files {
                let file_str = file.display().to_string();
                if let Some(research) = research {
                    if research.files_inspected.iter().any(|f| f == file) {
                        attached.push(format!("[research] {file_str} inspected by research"));
                    }
                }
                for entry in all_evidence.iter().filter(|e| e.reference == file_str) {
                    attached.push(format!("[{}] {}", entry.source, entry.summary));
                }
            }
            for symbol in &step.target_symbols {
                if let Some(research) = research {
                    if research.symbols_found.contains(symbol) {
                        attached.push(format!("[research] symbol {symbol} surfaced by research"));
                    }
                }
                for entry in all_evidence.iter().filter(|e| e.reference == *symbol) {
                    attached.push(format!("[{}] {}", entry.source, entry.summary));
                }
            }
            // The planner's own targeted reads of the target files.
            for observation in &self.observations {
                if observation.name == "read_file" {
                    if let Some(path) = parse_arg_path(&observation.arguments) {
                        if step
                            .target_files
                            .iter()
                            .any(|f| f.display().to_string() == path)
                        {
                            attached.push(format!(
                                "[planning_read] {} → {}",
                                path,
                                truncate_chars(&observation.result, 160)
                            ));
                        }
                    }
                }
            }
            // Authoritative testing command facts for validation commands.
            if let Some(testing) = testing {
                for command in &testing.commands_run {
                    if step.validation.iter().any(|v| v.contains(&command.command)) {
                        attached.push(format!(
                            "[testing] {} exit_code={} (authoritative)",
                            command.command, command.exit_code
                        ));
                    }
                }
            }
            attached.dedup();
            step.evidence = attached;
            linked.push(step);
        }
        linked
    }
}

impl PlanningResult {
    /// Record the provider/model that executed the planning.
    pub fn with_provider(mut self, provider: String, model: String) -> Self {
        self.provider = provider;
        self.model = model;
        self
    }
}

// =============================================================================
// Deterministic parsing helpers
// =============================================================================

/// Parse a `Step N: <action>` header. Matching is case-insensitive but the
/// extracted action preserves the model's original casing.
fn parse_step_header(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim();
    let lower = trimmed.to_lowercase();
    let (num_part, _) = lower.strip_prefix("step ")?.split_once(':')?;
    let order: usize = num_part.trim().parse().ok()?;
    let action = trimmed
        .split_once(':')?
        .1
        .trim()
        .trim_start_matches(['-', '*'])
        .trim()
        .to_string();
    if action.is_empty() {
        return None;
    }
    Some((order, action))
}

/// Strip a `Field:` prefix when the line starts with it (case-insensitive).
fn strip_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let lower = line.to_lowercase();
    if lower.starts_with(&field.to_lowercase()) {
        let rest = &line[field.len()..];
        let rest = rest.trim_start_matches([':', ' ', '\t']).trim();
        if rest.is_empty() {
            return None;
        }
        return Some(rest);
    }
    None
}

/// Parse file paths from a comma/whitespace separated field.
fn parse_paths(input: &str) -> Vec<PathBuf> {
    parse_tokens(input)
        .into_iter()
        .filter(|token| looks_like_path(token))
        .map(PathBuf::from)
        .collect()
}

/// Parse a validation field into its individual commands (split on `;`).
fn parse_commands(input: &str) -> Vec<String> {
    input
        .split(';')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

/// Split a field into tokens on commas and whitespace, stripping punctuation.
fn parse_tokens(input: &str) -> Vec<String> {
    input
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(|token| {
            token
                .trim_matches(['(', ')', '[', ']', ';', ':', ',', '\''])
                .trim()
                .to_string()
        })
        .filter(|token| !token.is_empty())
        .collect()
}

/// A token that looks like a file path (contains a separator or a known
/// source/build extension).
fn looks_like_path(token: &str) -> bool {
    token.contains('/')
        || token.contains('\\')
        || [
            "rs", "toml", "py", "ts", "js", "tsx", "jsx", "go", "md", "json", "yml", "yaml",
            "lock", "c", "h", "cpp", "hpp", "css", "html",
        ]
        .iter()
        .any(|ext| token.ends_with(&format!(".{ext}")))
}

/// A token that looks like an identifier or a path ending in a source file
/// (symbols may be qualified like `CanonicalRuntime::run_execution_loop`).
fn looks_like_identifier(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let cleaned = token.trim_matches(['(', ')', '[', ']', ';', ',']);
    let first = cleaned.chars().next().unwrap_or(' ');
    (first.is_alphabetic() || first == '_')
        && cleaned
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == ':' || c == '.')
}

fn union_paths(plan: &[PlanStep]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for step in plan {
        for file in &step.target_files {
            if !files.contains(file) {
                files.push(file.clone());
            }
        }
    }
    files
}

fn union_symbols(plan: &[PlanStep]) -> Vec<String> {
    let mut symbols = Vec::new();
    for step in plan {
        for symbol in &step.target_symbols {
            if !symbols.contains(symbol) {
                symbols.push(symbol.clone());
            }
        }
    }
    symbols
}

fn union_tests(plan: &[PlanStep]) -> Vec<PathBuf> {
    let mut tests = Vec::new();
    for step in plan {
        for file in &step.target_files {
            let name = file.display().to_string();
            if name.contains("test") && !tests.contains(file) {
                tests.push(file.clone());
            }
        }
    }
    tests
}

/// Parse the `path` argument from a tool-call JSON argument string.
fn parse_arg_path(arguments: &str) -> Option<String> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(path) = value.get("path").and_then(|v| v.as_str()) {
            return Some(path.to_string());
        }
    }
    Some(trimmed.trim_matches('"').to_string())
}

/// Extract file paths from a `list_files` result (one path per line).
fn list_output_paths(output: &str) -> Vec<PathBuf> {
    output
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .take(50)
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_arg_path_raw_and_json() {
        assert_eq!(
            parse_arg_path(r#"{"path": "src/main.rs"}"#),
            Some("src/main.rs".to_string())
        );
        assert_eq!(
            parse_arg_path("src/main.rs"),
            Some("src/main.rs".to_string())
        );
        assert_eq!(parse_arg_path(""), None);
    }

    #[test]
    fn test_parse_step_header_variants() {
        assert_eq!(
            parse_step_header("Step 1: Modify src/canonical_runtime/mod.rs"),
            Some((1, "Modify src/canonical_runtime/mod.rs".to_string()))
        );
        assert_eq!(
            parse_step_header("step 2: add a field"),
            Some((2, "add a field".to_string()))
        );
        assert_eq!(parse_step_header("Step: no number"), None);
        assert_eq!(parse_step_header("Step 3"), None);
    }

    #[test]
    fn test_strip_field_ignores_case() {
        assert_eq!(
            strip_field("Files: src/a.rs, src/b.rs", "Files:"),
            Some("src/a.rs, src/b.rs")
        );
        assert_eq!(
            strip_field("validate: cargo test", "Validate:"),
            Some("cargo test")
        );
        assert_eq!(strip_field("Symbols:", "Symbols:"), None);
        assert_eq!(strip_field("Reason: why", "Files:"), None);
    }

    #[test]
    fn test_parse_paths_and_tokens() {
        assert_eq!(
            parse_paths("src/a.rs, src/b.rs"),
            vec![PathBuf::from("src/a.rs"), PathBuf::from("src/b.rs")]
        );
        assert_eq!(parse_paths("cargo test --lib"), Vec::<PathBuf>::new());
        assert_eq!(
            parse_tokens("foo,foo bar"),
            vec!["foo".to_string(), "foo".to_string(), "bar".to_string()]
        );
    }

    #[test]
    fn test_looks_like_identifier() {
        assert!(looks_like_identifier("run_execution_loop"));
        assert!(looks_like_identifier("CanonicalRuntime::stream_once"));
        assert!(!looks_like_identifier("cargo test"));
        assert!(!looks_like_identifier(""));
    }
}
