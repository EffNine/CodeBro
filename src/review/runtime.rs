//! The autonomous Review subagent execution loop (Sprint 30G).
//!
//! This is a real executor, not a template generator. It receives the user
//! objective plus the structured evidence of Research, Testing, Planning and
//! Coding, independently inspects the actual repository state (via read-only
//! tools), compares intended changes against actual changes, evaluates
//! verification evidence, detects plan deviations and unverified changes, and
//! iterates until it produces a bounded, evidence-backed `ReviewResult`.
//!
//! ```text
//! ReviewRequest + GroundedContext + ResearchResult + TestingResult
//!      + PlanningResult + CodingResult
//!      ↓
//! ReviewSubagent loop
//!      ├── route provider (IntelligentProviderRouter)
//!      ├── stream via the shared canonical primitive (execution::stream_once)
//!      ├── structured / text tool-call parsing
//!      ├── read-only restricted registry (list_files / read_file / git_status / git_diff)
//!      ├── observation → next decision
//!      └── reserved final synthesis → ReviewResult
//!      ↓
//! ReviewResult
//! ```
//!
//! Safety contract:
//! - Review NEVER calls raw `fs::write`, `run_command`, `propose_change`, or
//!   any mutating tool. The restricted registry and the explicit permission
//!   hook enforce this boundary.
//! - Git history is NEVER mutated: only `git_status` and `git_diff` are
//!   allowed, both read-only.
//! - The machine owns repository facts — the model reasons over them but
//!   cannot override them.

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

use super::contract::{
    ReviewCategory, ReviewFinding, ReviewRequest, ReviewResult, ReviewSeverity, ReviewTermination,
    ReviewVerdict,
};
use super::limits::ReviewLimits;
use super::permissions::ReviewTooling;

/// The bounded review execution runtime.
pub struct ReviewSubagent {
    provider_runtime: ProviderRuntime,
    router: IntelligentProviderRouter,
    io_providers: HashMap<ProviderId, Arc<dyn crate::providers::Provider>>,
    tooling: ReviewTooling,
}

impl ReviewSubagent {
    /// Build a review subagent over the caller's shared provider state and a
    /// restricted read-only tool registry.
    pub fn new(
        provider_runtime: ProviderRuntime,
        router: IntelligentProviderRouter,
        io_providers: HashMap<ProviderId, Arc<dyn crate::providers::Provider>>,
        tooling: ReviewTooling,
    ) -> Self {
        ReviewSubagent {
            provider_runtime,
            router,
            io_providers,
            tooling,
        }
    }

    /// Run one bounded review session.
    pub async fn run(
        &mut self,
        request: ReviewRequest,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
        cancel: Option<CancellationToken>,
    ) -> ReviewResult {
        let started = Instant::now();
        let limits = request.limits.clone();
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(limits.timeout_ms);

        emit(AgentEvent::Log {
            level: "review".to_string(),
            message: format!("Review started: {}", request.task),
        });
        emit(AgentEvent::AgentStarted {
            agent: "review".to_string(),
            task: request.task.clone(),
        });
        emit(AgentEvent::AgentStatusChanged {
            agent: "review".to_string(),
            status: AgentStatus::Planning, // reuse Planning since Review is read-only analysis
        });

        let mut state = ReviewState::new(request, limits);
        let mut total_tool_calls = 0usize;

        loop {
            // 1. Cancellation.
            if let Some(token) = &cancel {
                if token.is_cancelled() {
                    return self.finish(state, ReviewTermination::Cancelled, started, emit);
                }
            }
            // 2. Deadline.
            if tokio::time::Instant::now() >= deadline {
                return self.finish(state, ReviewTermination::Timeout, started, emit);
            }
            // 3. Model-call budget.
            if state.model_calls >= state.limits.max_model_calls {
                return self.finish(state, ReviewTermination::ModelLimit, started, emit);
            }
            // 4. Iteration budget.
            if state.iterations >= state.limits.max_iterations {
                return self.finish(state, ReviewTermination::IterationLimit, started, emit);
            }

            // 5. Determine the phase: evidence gathering vs final synthesis.
            if !state.synthesis_attempted
                && state.model_calls >= state.limits.evidence_model_budget()
                && state.has_evidence()
            {
                state.synthesis_attempted = true;
                emit(AgentEvent::Log {
                    level: "review".to_string(),
                    message: "Review entering final synthesis phase".to_string(),
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
                level: "review".to_string(),
                message: format!("Review model call {}", state.model_calls + 1),
            });

            // 8. Execute through the shared canonical primitive.
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
                        parsed
                            .into_iter()
                            .map(|mut c| {
                                c.arguments = tool_parser::unwrap_tool_arguments(&c.arguments);
                                c
                            })
                            .collect()
                    };

                    // No tool call → the model produced its final report.
                    if calls.is_empty() {
                        state.final_answer = Some(full);
                        state.synthesis_complete = true;
                        let termination =
                            self.completion_gate(&mut state, cancel.clone(), emit).await;
                        let model = provider_model;
                        return self
                            .finish(state, termination, started, emit)
                            .with_provider(provider_id.as_str().to_string(), model);
                    }

                    // The reserved synthesis call must not keep exploring:
                    // terminate honestly — the evidence trail is preserved.
                    if state.synthesis_attempted {
                        return self.finish(state, ReviewTermination::ModelLimit, started, emit);
                    }

                    // 9. Tool-call budget.
                    if total_tool_calls + calls.len() > state.limits.max_tool_calls {
                        return self.finish(state, ReviewTermination::ToolLimit, started, emit);
                    }

                    for call in &calls {
                        total_tool_calls += 1;
                        state.tool_calls += 1;
                        emit(AgentEvent::ToolStarted {
                            tool: call.name.clone(),
                            args: call.arguments.clone(),
                        });
                        emit(AgentEvent::Log {
                            level: "review".to_string(),
                            message: format!(
                                "Review tool call {}: {}",
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
                            level: "review".to_string(),
                            message: format!(
                                "Review tool result {}: success={}",
                                state.tool_calls, success
                            ),
                        });
                        emit(AgentEvent::ToolCompleted {
                            tool: call.name.clone(),
                            result: truncated.clone(),
                            success,
                        });
                        state.observe(
                            call.name.clone(),
                            call.arguments.clone(),
                            truncated,
                            success,
                        );
                    }
                }
                Err(e) => {
                    return self.finish_error(state, e, started, emit);
                }
            }
        }
    }

    /// The final-answer completion gate: synthesize the findings and produce
    /// the verdict. If no changes were observed, synthesize a minimal review;
    /// otherwise synthesize the full evidence-backed report.
    async fn completion_gate(
        &mut self,
        state: &mut ReviewState,
        cancel: Option<CancellationToken>,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> ReviewTermination {
        // Synthesis already happened above; just finish with Completed.
        // The actual structured result assembly happens in `finish`.
        if state.changes_applied > 0 {
            emit(AgentEvent::Log {
                level: "review".to_string(),
                message: "Review completion gate: synthesizing findings".to_string(),
            });
        }
        ReviewTermination::Completed
    }

    /// Assemble the final result for a terminating session.
    fn finish(
        &self,
        state: ReviewState,
        termination: ReviewTermination,
        started: Instant,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> ReviewResult {
        if termination.is_completed() {
            emit(AgentEvent::AgentCompleted {
                agent: "review".to_string(),
                duration_ms: started.elapsed().as_millis() as u64,
            });
        } else {
            emit(AgentEvent::Log {
                level: "review".to_string(),
                message: format!("Review terminated: {}", termination),
            });
        }
        state.build_result(termination, started.elapsed().as_millis() as u64)
    }

    /// Assemble an error result for a session interrupted by a failure.
    fn finish_error(
        &mut self,
        mut state: ReviewState,
        error: String,
        started: Instant,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> ReviewResult {
        state.error = Some(error.clone());
        emit(AgentEvent::AgentFailed {
            agent: "review".to_string(),
            error: error.clone(),
        });
        emit(AgentEvent::Log {
            level: "review".to_string(),
            message: format!("Review failed: {}", error),
        });
        state.build_result(
            ReviewTermination::Error,
            started.elapsed().as_millis() as u64,
        )
    }
}

/// Accumulated review session state.
struct ReviewState {
    request: ReviewRequest,
    limits: ReviewLimits,
    iterations: usize,
    tool_calls: usize,
    model_calls: usize,
    synthesis_attempted: bool,
    synthesis_complete: bool,
    observations: Vec<ReviewObservation>,
    files_inspected: Vec<PathBuf>,
    final_answer: Option<String>,
    error: Option<String>,
    limitations: Vec<String>,
    /// The number of actual repository changes observed (coding or diff).
    changes_applied: usize,
}

impl ReviewState {
    fn new(request: ReviewRequest, limits: ReviewLimits) -> Self {
        ReviewState {
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
            limitations: Vec::new(),
            changes_applied: 0,
        }
    }

    fn observe(&mut self, name: String, arguments: String, result: String, success: bool) {
        // Track inspected files.
        if name == "read_file" {
            if let Some(path) = parse_arg_path(&arguments) {
                self.add_file(PathBuf::from(path));
            }
        }
        if name == "list_files" {
            for path in list_output_paths(&result) {
                self.add_file(path);
            }
        }
        self.observations.push(ReviewObservation {
            name,
            arguments,
            result,
            success,
        });
    }

    fn add_file(&mut self, path: PathBuf) {
        let path = self.relativize(path);
        if !self.files_inspected.contains(&path) {
            self.files_inspected.push(path);
        }
    }

    fn has_evidence(&self) -> bool {
        self.tool_calls > 0 || !self.observations.is_empty()
    }

    fn relativize(&self, path: PathBuf) -> PathBuf {
        if path.is_absolute() {
            path.strip_prefix(&self.request.workspace_root)
                .map(|p| p.to_path_buf())
                .unwrap_or(path)
        } else {
            path
        }
    }

    // =========================================================================
    // Prompts
    // =========================================================================

    fn build_prompt(&self) -> String {
        let mut prompt = String::new();
        prompt.push_str(
            "You are CodeBro's autonomous Review subagent. Your job is to independently inspect the repository state, compare intended changes against actual changes, evaluate verification evidence, detect plan deviations and unverified changes, and surface concrete, evidence-backed findings.\n\n",
        );
        prompt.push_str(&format!("USER OBJECTIVE:\n{}\n\n", self.request.task));

        // GROUNDING
        prompt.push_str("GROUNDING (initial repository knowledge):\n");
        prompt.push_str(&self.render_grounding());
        prompt.push('\n');

        // RESEARCH EVIDENCE
        prompt.push_str("RESEARCH EVIDENCE:\n");
        prompt.push_str(&self.render_research());
        prompt.push('\n');

        // TESTING EVIDENCE
        prompt.push_str("TESTING EVIDENCE:\n");
        prompt.push_str(&self.render_testing());
        prompt.push('\n');

        // PLAN EVIDENCE
        prompt.push_str("IMPLEMENTATION PLAN:\n");
        prompt.push_str(&self.render_plan());
        prompt.push('\n');

        // CODING EVIDENCE
        prompt.push_str("CODE CHANGES:\n");
        prompt.push_str(&self.render_coding());
        prompt.push('\n');

        // CURRENT OBSERVATIONS
        prompt.push_str("CURRENT OBSERVATIONS:\n");
        prompt.push_str(&self.render_observations());
        prompt.push('\n');

        prompt.push_str("\nAVAILABLE TOOLS:\n");
        for tool in self.request.limits.describe_tools() {
            prompt.push_str(&format!("- {}\n", tool));
        }

        prompt.push_str(&format!(
            "\nINSTRUCTIONS:\n1. Use read_file and list_files to independently inspect the repository state. Do NOT trust rendered prose — verify claims against actual file contents.\n2. Compare the plan's intended changes against the actual git diff / applied changes. Look for planned changes missing, unexpected changes, and unplanned files.\n3. Evaluate verification evidence: check whether every applied change has a successful verification record (exit_code == 0). Flag unverified changes.\n4. Look for security issues (permission bypass, secret exposure, unsafe filesystem access, shell injection, workspace boundary violations, credential writes).\n5. Identify regression risks (changed public contracts, broken call paths, error handling regressions, rollback problems).\n6. Be honest about limitations: if you cannot establish a fact, say so explicitly.\n7. You have a bounded execution budget: {} more evidence-gathering calls before the final synthesis, and {} total tool calls.\n8. When your evidence is sufficient, STOP calling tools and produce the final review report on your next turn.\n\nREVIEW STEP {}:\n",
            self.limits.evidence_model_budget().saturating_sub(self.model_calls).max(1),
            self.limits.max_tool_calls,
            self.iterations + 1,
        ));
        prompt
    }

    fn build_synthesis_prompt(&self) -> String {
        let mut prompt = String::new();
        prompt.push_str(
            "You are CodeBro's autonomous Review subagent final synthesis step. Produce the FINAL CODE REVIEW. No tools are available.\n\n",
        );
        prompt.push_str(&format!("USER OBJECTIVE:\n{}\n\n", self.request.task));
        prompt.push_str(&self.render_grounding());
        prompt.push('\n');
        prompt.push_str("RESEARCH EVIDENCE:\n");
        prompt.push_str(&self.render_research());
        prompt.push('\n');
        prompt.push_str("TESTING EVIDENCE:\n");
        prompt.push_str(&self.render_testing());
        prompt.push('\n');
        prompt.push_str("IMPLEMENTATION PLAN:\n");
        prompt.push_str(&self.render_plan());
        prompt.push('\n');
        prompt.push_str("CODE CHANGES:\n");
        prompt.push_str(&self.render_coding());
        prompt.push('\n');
        prompt.push_str("OBSERVATIONS:\n");
        prompt.push_str(&self.render_observations());
        prompt.push('\n');
        prompt.push_str(
            "\nINSTRUCTIONS:\n1. Synthesize the evidence above into a concise FINAL CODE REVIEW.\n2. Do NOT call any tools — this is the final synthesis step.\n3. Distinguish: confirmed issue, probable risk, limitation / insufficient evidence.\n4. Report: summary, findings (with severity, category, file, symbol, evidence, recommendation), verification status, plan adherence, regression risks, security concerns, remaining limitations.\n5. Emit an explicit verdict: PASS, PASS_WITH_RISKS, or FAIL.\n6. Never claim a change is verified unless an exit-code-0 machine verification actually covered it.\n\nFINAL CODE REVIEW FORMAT:\n## Summary\n<brief overview>\n\n## Findings\n<one per finding: severity, category, title, file, symbol, statement, evidence, recommendation>\n\n## Verification Status\n<per-file: verified or unverified>\n\n## Plan Adherence\n<planned vs actual, deviations>\n\n## Regression Risks\n<concrete risks with evidence>\n\n## Security Concerns\n<concerns with evidence, or none>\n\n## Limitations\n<what could not be established>\n\n## Verdict\n<PASS | PASS_WITH_RISKS | FAIL>\n\nFINAL CODE REVIEW:\n",
        );
        prompt
    }

    // =========================================================================
    // Renderers
    // =========================================================================

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
            for finding in research.findings.iter().take(10) {
                out.push_str(&format!(
                    "- {}{}\n",
                    finding.statement,
                    finding
                        .file
                        .as_ref()
                        .map(|f| format!(" (file: {})", f.display()))
                        .unwrap_or_default()
                ));
            }
        }
        if out.is_empty() {
            out.push_str("(research produced no evidence)\n");
        }
        out
    }

    fn render_testing(&self) -> String {
        let Some(testing) = &self.request.testing else {
            return "(no testing evidence available)\n".to_string();
        };
        let mut out = String::new();
        if testing.commands_run.is_empty() {
            out.push_str("(no validation commands were executed)\n");
        } else {
            out.push_str("Command results (AUTHORITATIVE machine facts):\n");
            for command in &testing.commands_run {
                out.push_str(&format!(
                    "- {} → exit_code: {}, success: {}{}{}{}\n",
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
                    },
                    if command.cancelled {
                        ", cancelled: true"
                    } else {
                        ""
                    }
                ));
            }
        }
        if !testing.failures.is_empty() {
            out.push_str("Failures:\n");
            for failure in testing.failures.iter().take(6) {
                out.push_str(&format!(
                    "- {} ({}) exit_code: {}\n",
                    failure.command,
                    failure.kind.as_str(),
                    failure.exit_code
                ));
            }
        }
        out
    }

    fn render_plan(&self) -> String {
        let Some(planning) = &self.request.planning else {
            return "(no implementation plan available)\n".to_string();
        };
        let mut out = String::new();
        out.push_str(&format!(
            "Plan summary: {}\n",
            truncate_chars(&planning.summary, 400)
        ));
        if !planning.affected_files.is_empty() {
            out.push_str(&format!(
                "Affected files (the plan-adherence boundary): {}\n",
                planning
                    .affected_files
                    .iter()
                    .map(|f| f.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !planning.affected_symbols.is_empty() {
            out.push_str(&format!(
                "Affected symbols: {}\n",
                planning.affected_symbols.join(", ")
            ));
        }
        if !planning.tests_to_update.is_empty() {
            out.push_str(&format!(
                "Tests to update: {}\n",
                planning
                    .tests_to_update
                    .iter()
                    .map(|f| f.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if planning.plan.is_empty() {
            out.push_str("(no concrete steps)\n");
        } else {
            out.push_str("Plan steps:\n");
            for step in &planning.plan {
                out.push_str(&format!("{}. {}\n", step.order, step.action));
                if !step.target_files.is_empty() {
                    out.push_str(&format!(
                        "   Files: {}\n",
                        step.target_files
                            .iter()
                            .map(|f| f.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !step.validation.is_empty() {
                    out.push_str(&format!("   Validate: {}\n", step.validation.join("; ")));
                }
                if !step.risk.is_empty() {
                    out.push_str(&format!("   Risk: {}\n", step.risk));
                }
            }
        }
        out
    }

    fn render_coding(&self) -> String {
        let Some(coding) = &self.request.coding else {
            return "(no coding result available)\n".to_string();
        };
        let mut out = String::new();
        if coding.changes.is_empty() {
            out.push_str("(no changes applied)\n");
        } else {
            for change in &coding.changes {
                out.push_str(&format!(
                    "- {} [{}]{}\n",
                    change.path.display(),
                    change.status(),
                    if change.unplanned { " [UNPLANNED]" } else { "" }
                ));
                for line in change.preview.lines().take(12) {
                    out.push_str(&format!("  {}\n", line));
                }
            }
        }
        if !coding.unplanned_changes.is_empty() {
            out.push_str("\nUnplanned changes (deviation from the plan):\n");
            for change in &coding.unplanned_changes {
                out.push_str(&format!(
                    "- {} [{}]\n",
                    change.path.display(),
                    change.status()
                ));
            }
        }
        if !coding.verification.is_empty() {
            out.push_str("\nVerification records:\n");
            for record in &coding.verification {
                out.push_str(&format!(
                    "- {} (source: {}) → exit_code: {}, success: {}{}{}\n",
                    record.command,
                    record.source,
                    record.exit_code,
                    record.success,
                    if record.denied {
                        format!(
                            ", denied: {}",
                            record.denied_reason.as_deref().unwrap_or("")
                        )
                    } else {
                        String::new()
                    },
                    if record.timeout {
                        ", timed_out: true"
                    } else {
                        ""
                    }
                ));
            }
        }
        if coding.termination.is_completed() {
            out.push_str("\nCoding termination: completed (machine-verified)\n");
        } else {
            out.push_str(&format!(
                "\nCoding termination: {} (NOT completed-as-verified)\n",
                coding.termination
            ));
        }
        out
    }

    fn render_observations(&self) -> String {
        if self.observations.is_empty() {
            return "(none yet)\n".to_string();
        }
        let mut out = String::new();
        for (i, obs) in self.observations.iter().enumerate() {
            out.push_str(&format!(
                "  {}. {} {} → success={}\n     {}\n",
                i + 1,
                obs.name,
                truncate_chars(&obs.arguments, 200),
                obs.success,
                truncate_chars(&obs.result, 400)
            ));
        }
        out
    }

    // =========================================================================
    // Result assembly
    // =========================================================================

    fn build_result(&self, termination: ReviewTermination, duration_ms: u64) -> ReviewResult {
        let summary = self
            .final_answer
            .clone()
            .unwrap_or_else(|| self.default_summary(termination));
        let limitations = self.build_limitations(termination);
        let output_size = self.estimate_output();

        // Compute structured fields from evidence.
        let coding = self.request.coding.clone();
        let planned: Vec<PathBuf> = coding
            .as_ref()
            .and_then(|c| {
                self.request
                    .planning
                    .as_ref()
                    .map(|p| p.affected_files.clone())
            })
            .unwrap_or_default();
        let actual: Vec<PathBuf> = coding
            .as_ref()
            .map(|c| c.changes.iter().map(|ch| ch.path.clone()).collect())
            .unwrap_or_default();
        let verified: Vec<PathBuf> = coding
            .as_ref()
            .map(|c| {
                c.changes
                    .iter()
                    .filter(|ch| ch.verified)
                    .map(|ch| ch.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        let unverified: Vec<PathBuf> = coding
            .as_ref()
            .map(|c| {
                c.changes
                    .iter()
                    .filter(|ch| !ch.verified && !ch.rolled_back)
                    .map(|ch| ch.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        let deviations: Vec<PathBuf> = coding
            .as_ref()
            .map(|c| {
                c.unplanned_changes
                    .iter()
                    .map(|ch| ch.path.clone())
                    .collect()
            })
            .unwrap_or_default();

        let findings = parse_findings(&self.final_answer.as_deref().unwrap_or(""));
        let verdict = parse_verdict(
            &self.final_answer.as_deref().unwrap_or(""),
            &findings,
            &unverified,
            &deviations,
        );
        let security_concerns =
            parse_security_concerns(&self.final_answer.as_deref().unwrap_or(""));
        let regression_risks = parse_regression_risks(&self.final_answer.as_deref().unwrap_or(""));

        ReviewResult {
            summary,
            findings,
            reviewed_files: self.files_inspected.clone(),
            changed_files: actual.clone(),
            planned_changes: planned.clone(),
            actual_changes: actual,
            verified_changes: verified,
            unverified_changes: unverified,
            plan_deviations: deviations,
            security_concerns,
            regression_risks,
            tool_calls: self.tool_calls,
            iterations: self.iterations,
            model_calls: self.model_calls,
            termination,
            synthesis_complete: self.synthesis_complete,
            limitations,
            duration_ms,
            output_size,
            provider: String::new(),
            model: String::new(),
            verdict,
        }
    }

    fn default_summary(&self, termination: ReviewTermination) -> String {
        format!(
            "Review terminated with status '{}' after {} iteration(s), {} tool call(s), {} model call(s).",
            termination,
            self.iterations,
            self.tool_calls,
            self.model_calls
        )
    }

    fn build_limitations(&self, termination: ReviewTermination) -> Vec<String> {
        let mut limitations = self.limitations.clone();
        if let Some(error) = &self.error {
            limitations.push(error.clone());
        }
        match termination {
            ReviewTermination::Completed => {}
            ReviewTermination::IterationLimit => {
                limitations.push("iteration limit reached".to_string());
            }
            ReviewTermination::ToolLimit => {
                limitations.push("tool-call limit reached".to_string());
            }
            ReviewTermination::ModelLimit => {
                limitations
                    .push("model-call limit reached; final synthesis incomplete".to_string());
            }
            ReviewTermination::Timeout => {
                limitations.push("review timeout reached".to_string());
            }
            ReviewTermination::Cancelled => {
                limitations.push("review cancelled".to_string());
            }
            ReviewTermination::Error => {}
        }
        limitations
    }

    fn estimate_output(&self) -> usize {
        let mut size = 0usize;
        for obs in &self.observations {
            size += obs.name.len() + obs.arguments.len() + obs.result.len();
        }
        size + self.final_answer.clone().unwrap_or_default().len()
    }
}

/// One real tool observation performed during review.
#[derive(Debug, Clone)]
struct ReviewObservation {
    name: String,
    arguments: String,
    result: String,
    success: bool,
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
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .take(50)
        .map(PathBuf::from)
        .collect()
}

// =========================================================================
// Synthesis prose parsing — best-effort extraction of structured fields
// from the model's final-code-review output. These are deterministic,
// bounded parsers; they never panic and fall back to safe defaults when
// the prose is malformed or missing.
// =========================================================================

/// Extract the verdict from synthesis prose.
///
/// The synthesis prompt instructs the model to emit `## Verdict` followed by
/// `PASS`, `PASS_WITH_RISKS`, or `FAIL`. The verdict is parsed ONLY from that
/// `## Verdict` section, using standalone-token matching — body prose such as
/// "no failures found" or "fails at runtime" must never be misread as a FAIL
/// verdict (real-provider smoke exposed exactly this false positive). When
/// the section is missing, or when authoritative machine facts contradict a
/// `PASS`, the verdict is downgraded conservatively.
///
/// Authoritative downgrade invariants (model prose must never override them):
/// - unverified changes, plan deviations or a Critical finding make `PASS`
///   impossible (downgraded to at most `PassWithRisks`; an explicit `FAIL`
///   stays `Fail`);
/// - a missing verdict falls back to `PassWithRisks` (never `Pass`).
fn parse_verdict(
    text: &str,
    findings: &[ReviewFinding],
    unverified: &[PathBuf],
    deviations: &[PathBuf],
) -> ReviewVerdict {
    let explicit = extract_verdict_section(text)
        .as_deref()
        .and_then(verdict_token);

    // Machine facts always win: unverified changes, plan deviations and
    // Critical findings make PASS impossible. Only an explicit FAIL token in
    // the Verdict section keeps the verdict at Fail.
    if !unverified.is_empty() || !deviations.is_empty() || has_critical_finding(findings) {
        if explicit == Some(ReviewVerdict::Fail) {
            return ReviewVerdict::Fail;
        }
        return ReviewVerdict::PassWithRisks;
    }

    // No authoritative contradiction: the explicit token wins; a missing
    // verdict defaults conservatively to PassWithRisks (never Pass).
    explicit.unwrap_or(ReviewVerdict::PassWithRisks)
}

/// Extract the `## Verdict` section: the lines after the `## Verdict` header
/// until the next `## ` header (or the end of the text). Returns `None` when
/// the section is absent.
fn extract_verdict_section(text: &str) -> Option<String> {
    let mut in_section = false;
    let mut section = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if in_section {
            if trimmed.starts_with("## ") {
                break;
            }
            section.push_str(line);
            section.push('\n');
        } else if trimmed.to_lowercase().starts_with("## verdict") {
            in_section = true;
        }
    }
    if in_section {
        Some(section)
    } else {
        None
    }
}

/// The verdict token explicitly written in the `## Verdict` section, if any.
/// Standalone-token matching: "No failures found" is NOT a FAIL verdict.
fn verdict_token(section: &str) -> Option<ReviewVerdict> {
    let lower = section.to_lowercase();
    if lower.contains("pass_with_risks") || lower.contains("pass with risks") {
        return Some(ReviewVerdict::PassWithRisks);
    }
    if contains_standalone_token(section, "fail") {
        return Some(ReviewVerdict::Fail);
    }
    if contains_standalone_token(section, "pass") {
        return Some(ReviewVerdict::Pass);
    }
    None
}

/// Whether `token` appears in `text` as a standalone word (case-insensitive).
fn contains_standalone_token(text: &str, token: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .any(|word| word.eq_ignore_ascii_case(token))
}

/// Whether any finding is severity Critical. A Critical finding is a machine-
/// surfaced contradiction: `PASS` is never acceptable alongside one.
fn has_critical_finding(findings: &[ReviewFinding]) -> bool {
    findings
        .iter()
        .any(|f| f.severity == ReviewSeverity::Critical)
}

/// Parse structured findings from synthesis prose.
///
/// Expected format (one per finding):
/// ```text
/// - [severity] category — title
///   file: <path>
///   symbol: <name>
///   statement: <text>
///   evidence: <tool observation or machine fact>
///   recommendation: <text>
/// ```
fn parse_findings(text: &str) -> Vec<ReviewFinding> {
    let mut findings = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        // Match a finding header: "- [severity] category — title"
        if line.starts_with("- [") {
            if let Some(rest) = line.strip_prefix("- [") {
                if let Some(sev_end) = rest.find(']') {
                    let (sev_str, after_sev) = rest.split_at(sev_end + 1);
                    if after_sev.trim().starts_with(char::is_alphabetic) {
                        let severity = match sev_str
                            .trim()
                            .strip_suffix(']')
                            .unwrap_or(sev_str.trim())
                            .to_lowercase()
                            .as_str()
                        {
                            "critical" => ReviewSeverity::Critical,
                            "high" => ReviewSeverity::High,
                            "medium" => ReviewSeverity::Medium,
                            "low" => ReviewSeverity::Low,
                            _ => ReviewSeverity::Info,
                        };
                        // Collect continuation lines until the next finding or section.
                        let mut title = after_sev.trim().to_string();
                        let mut file: Option<PathBuf> = None;
                        let mut symbol: Option<String> = None;
                        let mut statement = String::new();
                        let mut evidence = String::new();
                        let mut recommendation = String::new();
                        i += 1;
                        while i < lines.len() {
                            let cont = lines[i].trim();
                            if cont.starts_with("- [") {
                                break;
                            }
                            if cont.starts_with("file:") {
                                file = Some(PathBuf::from(
                                    cont.strip_prefix("file:").unwrap_or("").trim(),
                                ));
                            } else if cont.starts_with("symbol:") {
                                symbol = Some(
                                    cont.strip_prefix("symbol:")
                                        .unwrap_or("")
                                        .trim()
                                        .to_string(),
                                );
                            } else if cont.starts_with("statement:") {
                                statement = cont
                                    .strip_prefix("statement:")
                                    .unwrap_or("")
                                    .trim()
                                    .to_string();
                            } else if cont.starts_with("evidence:") {
                                evidence = cont
                                    .strip_prefix("evidence:")
                                    .unwrap_or("")
                                    .trim()
                                    .to_string();
                            } else if cont.starts_with("recommendation:") {
                                recommendation = cont
                                    .strip_prefix("recommendation:")
                                    .unwrap_or("")
                                    .trim()
                                    .to_string();
                            } else if !cont.is_empty()
                                && statement.is_empty()
                                && !title.contains(" — ")
                            {
                                // Continuation of the title line.
                                title.push_str(" ");
                                title.push_str(cont);
                            }
                            i += 1;
                        }
                        // Only emit if we got at least a statement or evidence.
                        if !statement.is_empty() || !evidence.is_empty() {
                            findings.push(ReviewFinding {
                                severity,
                                category: ReviewCategory::Correctness,
                                title,
                                file,
                                symbol,
                                statement,
                                evidence,
                                recommendation,
                            });
                        }
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    findings
}

/// Extract security concerns from synthesis prose.
fn parse_security_concerns(text: &str) -> Vec<String> {
    let mut concerns = Vec::new();
    let in_section = {
        let lower = text.to_lowercase();
        lower.find("## security concerns").unwrap_or(text.len())
            < lower.find("## limitations").unwrap_or(text.len())
    };
    if !in_section {
        return concerns;
    }
    let lines = text.lines();
    let mut collecting = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.to_lowercase().starts_with("## security concerns") {
            collecting = true;
            continue;
        }
        if trimmed.starts_with("## ") && collecting {
            break;
        }
        if collecting && trimmed.starts_with("- ") {
            concerns.push(trimmed.strip_prefix("- ").unwrap_or(trimmed).to_string());
        }
    }
    concerns
}

/// Extract regression risks from synthesis prose.
fn parse_regression_risks(text: &str) -> Vec<String> {
    let mut risks = Vec::new();
    let in_section = {
        let lower = text.to_lowercase();
        lower.find("## regression risks").unwrap_or(text.len())
            < lower.find("## security concerns").unwrap_or(text.len())
    };
    if !in_section {
        return risks;
    }
    let lines = text.lines();
    let mut collecting = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.to_lowercase().starts_with("## regression risks") {
            collecting = true;
            continue;
        }
        if trimmed.starts_with("## ") && collecting {
            break;
        }
        if collecting && trimmed.starts_with("- ") {
            risks.push(trimmed.strip_prefix("- ").unwrap_or(trimmed).to_string());
        }
    }
    risks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_verdict_from_pass_synthesis() {
        let text = "## Verdict\nPASS\n";
        assert_eq!(parse_verdict(text, &[], &[], &[]), ReviewVerdict::Pass);
    }

    #[test]
    fn test_parse_verdict_from_fail_synthesis() {
        let text = "## Verdict\nFAIL\n";
        assert_eq!(parse_verdict(text, &[], &[], &[]), ReviewVerdict::Fail);
    }

    #[test]
    fn test_parse_verdict_from_pass_with_risks_synthesis() {
        let text = "## Verdict\nPASS_WITH_RISKS\n";
        assert_eq!(
            parse_verdict(text, &[], &[], &[]),
            ReviewVerdict::PassWithRisks
        );
    }

    #[test]
    fn test_parse_verdict_downgrades_pass_when_unverified_changes_exist() {
        let text = "## Verdict\nPASS\n";
        let unverified = vec![PathBuf::from("src/lib.rs")];
        assert_eq!(
            parse_verdict(text, &[], &unverified, &[]),
            ReviewVerdict::PassWithRisks
        );
    }

    #[test]
    fn test_parse_verdict_downgrades_pass_when_deviations_exist() {
        let text = "## Verdict\nPASS\n";
        let deviations = vec![PathBuf::from("src/extra.rs")];
        assert_eq!(
            parse_verdict(text, &[], &[], &deviations),
            ReviewVerdict::PassWithRisks
        );
    }

    #[test]
    fn test_parse_verdict_downgrades_pass_when_critical_finding_exists() {
        // A Critical finding must never be hidden behind a PASS verdict, even
        // when the model's prose explicitly claims PASS (audit invariant).
        let text = "## Verdict\nPASS\n";
        let critical = vec![ReviewFinding {
            severity: ReviewSeverity::Critical,
            category: ReviewCategory::Security,
            title: "hardcoded secret".to_string(),
            file: Some(PathBuf::from("src/config.rs")),
            symbol: None,
            statement: "credential is hardcoded".to_string(),
            evidence: "read_file showed plaintext".to_string(),
            recommendation: String::new(),
        }];
        assert_eq!(
            parse_verdict(text, &critical, &[], &[]),
            ReviewVerdict::PassWithRisks
        );
        // An explicit FAIL alongside a critical finding stays FAIL.
        assert_eq!(
            parse_verdict("## Verdict\nFAIL\n", &critical, &[], &[]),
            ReviewVerdict::Fail
        );
    }

    #[test]
    fn test_parse_verdict_ignores_non_critical_findings_for_pass() {
        // A low-severity or informational finding does not block a PASS.
        let text = "## Verdict\nPASS\n";
        let low = vec![ReviewFinding {
            severity: ReviewSeverity::Low,
            category: ReviewCategory::Maintainability,
            title: "style nit".to_string(),
            file: None,
            symbol: None,
            statement: "minor".to_string(),
            evidence: "observed".to_string(),
            recommendation: String::new(),
        }];
        assert_eq!(parse_verdict(text, &low, &[], &[]), ReviewVerdict::Pass);
    }

    #[test]
    fn test_parse_verdict_allows_fail_even_with_unverified() {
        let text = "## Verdict\nFAIL\n";
        let unverified = vec![PathBuf::from("src/lib.rs")];
        assert_eq!(
            parse_verdict(text, &[], &unverified, &[]),
            ReviewVerdict::Fail
        );
    }

    #[test]
    fn test_parse_verdict_falls_back_to_pass_with_risks_when_missing() {
        let text = "No verdict section present.\n";
        assert_eq!(
            parse_verdict(text, &[], &[], &[]),
            ReviewVerdict::PassWithRisks
        );
    }

    #[test]
    fn test_parse_verdict_ignores_body_prose_fail_words() {
        // Real-provider smoke (Sprint 31A): body prose like "no failures
        // found" must never be misread as a FAIL verdict. The verdict comes
        // exclusively from the ## Verdict section.
        let text = "## Summary\nNo failures found; the change is correct.\n\
                    ## Verdict\nPASS\n";
        assert_eq!(parse_verdict(text, &[], &[], &[]), ReviewVerdict::Pass);

        // Even "fails at runtime" in the body must not flip the verdict.
        let text = "## Summary\nNothing fails at runtime.\n## Verdict\nPASS\n";
        assert_eq!(parse_verdict(text, &[], &[], &[]), ReviewVerdict::Pass);
    }

    #[test]
    fn test_parse_verdict_uses_only_the_verdict_section() {
        // A FAIL token inside the Verdict section is authoritative...
        let text = "## Summary\nThe change compiles.\n## Verdict\nFAIL\n";
        assert_eq!(parse_verdict(text, &[], &[], &[]), ReviewVerdict::Fail);

        // ...while an early section boundary cuts off later prose.
        let text = "## Verdict\nPASS\n## Findings\n- [high] regression — callers may break\n";
        assert_eq!(parse_verdict(text, &[], &[], &[]), ReviewVerdict::Pass);

        // Standalone-token matching: "No failures found" inside the Verdict
        // section is not a FAIL token either.
        let text = "## Verdict\nNo failures found\n";
        assert_eq!(
            parse_verdict(text, &[], &[], &[]),
            ReviewVerdict::PassWithRisks
        );
    }

    #[test]
    fn test_parse_verdict_case_insensitive_tokens() {
        assert_eq!(
            parse_verdict("## Verdict\nfail\n", &[], &[], &[]),
            ReviewVerdict::Fail
        );
        assert_eq!(
            parse_verdict("## Verdict\npass\n", &[], &[], &[]),
            ReviewVerdict::Pass
        );
        assert_eq!(
            parse_verdict("## Verdict\npass_with_risks\n", &[], &[], &[]),
            ReviewVerdict::PassWithRisks
        );
    }

    #[test]
    fn test_parse_findings_basic() {
        let text = r#"## Findings
- [high] verification — unverified change
  file: src/lib.rs
  symbol: add
  statement: no machine verification covered this change
  evidence: exit_code: -1, success: false
  recommendation: run cargo check
- [low] correctness — minor style issue
  file: src/main.rs
  statement: trailing whitespace
  evidence: read_file showed trailing space on line 5
  recommendation: remove whitespace
## Verdict
PASS"#;
        let findings = parse_findings(text);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].severity, ReviewSeverity::High);
        assert_eq!(
            findings[0].statement,
            "no machine verification covered this change"
        );
        assert_eq!(findings[0].evidence, "exit_code: -1, success: false");
        assert_eq!(findings[1].severity, ReviewSeverity::Low);
    }

    #[test]
    fn test_parse_findings_empty_when_no_section() {
        let text = "Just some prose without findings.\n";
        assert!(parse_findings(text).is_empty());
    }

    #[test]
    fn test_parse_findings_ignores_incomplete_entries() {
        // A finding header without statement or evidence must be skipped.
        let text = "- [high] correctness — title only\n  file: src/x.rs\n";
        assert!(parse_findings(text).is_empty());
    }

    #[test]
    fn test_parse_security_concerns() {
        let text = r#"## Security Concerns
- hardcoded secret in src/config.rs
- no input sanitization
## Limitations
none"#;
        let concerns = parse_security_concerns(text);
        assert_eq!(concerns.len(), 2);
        assert!(concerns[0].contains("hardcoded secret"));
    }

    #[test]
    fn test_parse_security_concerns_empty_when_no_section() {
        let text = "## Summary\nnothing\n## Verdict\nPASS\n";
        assert!(parse_security_concerns(text).is_empty());
    }

    #[test]
    fn test_parse_regression_risks() {
        let text = r#"## Regression Risks
- callers of add() may break
- signature change affects tests
## Security Concerns
none"#;
        let risks = parse_regression_risks(text);
        assert_eq!(risks.len(), 2);
        assert!(risks[0].contains("callers"));
    }

    #[test]
    fn test_parse_regression_risks_empty_when_no_section() {
        let text = "## Summary\nnothing\n## Verdict\nPASS\n";
        assert!(parse_regression_risks(text).is_empty());
    }

    #[test]
    fn test_list_output_paths() {
        let paths = list_output_paths("a.rs\nb.rs\n");
        assert_eq!(paths, vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")]);
    }

    #[test]
    fn test_render_testing_preserves_cancelled_machine_fact() {
        // A cancelled testing command must keep its machine fact visible to
        // the reviewer: exit code -1, success false AND the cancelled marker.
        let mut state = ReviewState::new(
            ReviewRequest::new("review the work", "."),
            ReviewLimits::default(),
        );
        let cancelled = crate::testing::TestCommandResult {
            command: "cargo test".to_string(),
            working_directory: "/r".to_string(),
            exit_code: -1,
            success: false,
            duration_ms: 100,
            output: String::new(),
            timeout: false,
            cancelled: true,
            denied: false,
            denied_reason: None,
        };
        state.request.testing = Some(crate::testing::TestingResult {
            summary: String::new(),
            findings: Vec::new(),
            commands_run: vec![cancelled],
            files_inspected: Vec::new(),
            failures: Vec::new(),
            tool_calls: 1,
            iterations: 1,
            model_calls: 1,
            termination: crate::testing::TestingTermination::Cancelled,
            synthesis_complete: false,
            observations: Vec::new(),
            limitations: Vec::new(),
            duration_ms: 0,
            output_size: 0,
            provider: String::new(),
            model: String::new(),
            git_before: None,
            git_after: None,
        });
        let rendered = state.render_testing();
        assert!(
            rendered.contains("exit_code: -1"),
            "exit code must stay visible: {rendered}"
        );
        assert!(
            rendered.contains("success: false"),
            "success must stay visible: {rendered}"
        );
        assert!(
            rendered.contains("cancelled: true"),
            "the cancelled machine fact must not be dropped: {rendered}"
        );
    }
}
