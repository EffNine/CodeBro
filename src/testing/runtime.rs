//! The autonomous Testing subagent execution loop (Sprint 30D).
//!
//! This is a real executor, not a template generator. It receives an
//! objective, decides which bounded validation command to run, executes it
//! through the policy-checked registry, observes the authoritative exit code,
//! decides the next action, and iterates until it produces a bounded,
//! evidence-backed `TestingResult`.
//!
//! ```text
//! TestingRequest + GroundedContext
//!      ↓
//! TestingSubagent loop
//!      ├── git-state snapshot (before)
//!      ├── route provider (IntelligentProviderRouter)
//!      ├── stream via the shared canonical primitive (execution::stream_once)
//!      ├── structured / text tool-call parsing
//!      ├── policy-checked restricted registry (run_command + read-only tools)
//!      ├── authoritative exit-code observation
//!      └── git-state snapshot (after) → no-mutation proof
//!      ↓
//! TestingResult
//! ```
//!
//! The execution result belongs to the machine: `success` comes from the
//! process exit code, never from output prose. The model interprets the
//! result; it cannot override it.

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
    GitStateSnapshot, TestCommandResult, TestFailure, TestFailureKind, TestFinding,
    TestObservation, TestingRequest, TestingResult, TestingTermination,
};
use super::permissions::TestingTooling;

/// The bounded testing execution runtime.
pub struct TestingSubagent {
    provider_runtime: ProviderRuntime,
    router: IntelligentProviderRouter,
    io_providers: HashMap<ProviderId, Arc<dyn crate::providers::Provider>>,
    tooling: TestingTooling,
}

impl TestingSubagent {
    /// Build a testing subagent over the caller's shared provider state and a
    /// restricted, policy-checked tool registry. All components are reused
    /// from the canonical runtime — nothing is re-implemented.
    pub fn new(
        provider_runtime: ProviderRuntime,
        router: IntelligentProviderRouter,
        io_providers: HashMap<ProviderId, Arc<dyn crate::providers::Provider>>,
        tooling: TestingTooling,
    ) -> Self {
        TestingSubagent {
            provider_runtime,
            router,
            io_providers,
            tooling,
        }
    }

    /// Run one bounded testing session.
    pub async fn run(
        &mut self,
        request: TestingRequest,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
        cancel: Option<CancellationToken>,
    ) -> TestingResult {
        let started = Instant::now();
        let limits = request.limits.clone();
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(limits.timeout_ms);

        emit(AgentEvent::Log {
            level: "testing".to_string(),
            message: format!("Testing started: {}", request.task),
        });
        emit(AgentEvent::AgentStarted {
            agent: "testing".to_string(),
            task: request.task.clone(),
        });
        emit(AgentEvent::AgentStatusChanged {
            agent: "testing".to_string(),
            status: AgentStatus::Testing,
        });

        let mut state = TestingState::new(request, limits);
        let mut total_tool_calls = 0usize;

        // No-mutation protocol: snapshot git state before execution so the
        // final result can prove the tracked tree was left untouched.
        state.git_before = Some(self.tooling.check_git_state());

        loop {
            // 1. Cancellation.
            if let Some(token) = &cancel {
                if token.is_cancelled() {
                    return self.finish(state, TestingTermination::Cancelled, started, emit);
                }
            }
            // 2. Deadline.
            if tokio::time::Instant::now() >= deadline {
                return self.finish(state, TestingTermination::Timeout, started, emit);
            }
            // 3. Model-call budget.
            if state.model_calls >= state.limits.max_model_calls {
                return self.finish(state, TestingTermination::ModelLimit, started, emit);
            }
            // 4. Iteration budget.
            if state.iterations >= state.limits.max_iterations {
                return self.finish(state, TestingTermination::IterationLimit, started, emit);
            }

            // 5. Determine the phase: evidence gathering vs final synthesis.
            //    The loop reserves one model call for synthesis so a model that
            //    keeps running commands can never starve the final report.
            if !state.synthesis_attempted
                && state.model_calls >= state.limits.evidence_model_budget()
                && state.has_evidence()
            {
                state.synthesis_attempted = true;
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
                level: "testing".to_string(),
                message: format!("Testing model call {}", state.model_calls + 1),
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

                    // No tool call → the model produced its final report. This
                    // is the synthesis-complete signal: the model stopped
                    // running commands and wrote a prose summary.
                    if calls.is_empty() {
                        state.final_answer = Some(full);
                        state.synthesis_complete = true;
                        let model = provider_model;
                        return self
                            .finish(state, TestingTermination::Completed, started, emit)
                            .with_provider(provider_id.as_str().to_string(), model);
                    }

                    // The reserved synthesis call must not keep running
                    // commands: terminate honestly — the structured command
                    // evidence gathered so far is preserved and no summary is
                    // fabricated.
                    if state.synthesis_attempted {
                        return self.finish(state, TestingTermination::ModelLimit, started, emit);
                    }

                    // 9. Tool-call budget.
                    if total_tool_calls + calls.len() > state.limits.max_tool_calls {
                        return self.finish(state, TestingTermination::ToolLimit, started, emit);
                    }

                    for call in &calls {
                        total_tool_calls += 1;
                        state.tool_calls += 1;
                        emit(AgentEvent::ToolStarted {
                            tool: call.name.clone(),
                            args: crate::tools::shell::redact_secrets_public(&call.arguments),
                        });
                        emit(AgentEvent::Log {
                            level: "testing".to_string(),
                            message: format!(
                                "Testing tool call {}: {}",
                                state.tool_calls, call.name
                            ),
                        });

                        if call.name == "run_command" {
                            let record = self
                                .tooling
                                .execute_command(&call.arguments, cancel.clone())
                                .await;
                            let truncated = truncate_chars(
                                &record.render(),
                                state.limits.max_command_output_chars,
                            );
                            emit(AgentEvent::Log {
                                level: "testing".to_string(),
                                message: format!(
                                    "Testing command completed: exit={} success={} denied={} timeout={}",
                                    record.exit_code, record.success, record.denied, record.timeout
                                ),
                            });
                            emit(AgentEvent::ToolCompleted {
                                tool: call.name.clone(),
                                result: truncated.clone(),
                                success: record.success,
                            });
                            state.observe_command(record);
                        } else {
                            let result = self
                                .tooling
                                .execute_tool(&call.name, &call.arguments, cancel.clone())
                                .await;
                            let truncated =
                                truncate_chars(&result, state.limits.max_command_output_chars);
                            let success = !result.starts_with("Error:");
                            emit(AgentEvent::Log {
                                level: "testing".to_string(),
                                message: format!(
                                    "Testing tool result {}: success={}",
                                    state.tool_calls, success
                                ),
                            });
                            emit(AgentEvent::ToolCompleted {
                                tool: call.name.clone(),
                                result: truncated.clone(),
                                success,
                            });
                            state.observe_tool(TestObservation {
                                name: call.name.clone(),
                                arguments: call.arguments.clone(),
                                result: truncated,
                                success,
                            });
                        }
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
        &mut self,
        mut state: TestingState,
        termination: TestingTermination,
        started: Instant,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> TestingResult {
        // No-mutation protocol: snapshot git state after execution. A change
        // in the tracked tree is surfaced as a limitation (defense in depth —
        // the policy already prevents mutation).
        state.git_after = Some(self.tooling.check_git_state());
        if !state.git_tree_unchanged() {
            state.limitations.push(
                "git tracked state changed during testing — unexpected mutation detected"
                    .to_string(),
            );
        }

        if termination.is_completed() {
            emit(AgentEvent::AgentCompleted {
                agent: "testing".to_string(),
                duration_ms: started.elapsed().as_millis() as u64,
            });
        } else {
            emit(AgentEvent::Log {
                level: "testing".to_string(),
                message: format!("Testing terminated: {}", termination),
            });
        }
        state.build_result(termination, started.elapsed().as_millis() as u64)
    }

    /// Assemble an error result for a session interrupted by a failure.
    fn finish_error(
        &mut self,
        mut state: TestingState,
        error: String,
        started: Instant,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> TestingResult {
        state.error = Some(error.clone());
        emit(AgentEvent::AgentFailed {
            agent: "testing".to_string(),
            error: error.clone(),
        });
        emit(AgentEvent::Log {
            level: "testing".to_string(),
            message: format!("Testing failed: {}", error),
        });
        state.git_after = Some(self.tooling.check_git_state());
        state.build_result(
            TestingTermination::Error,
            started.elapsed().as_millis() as u64,
        )
    }
}

/// Accumulated testing session state.
struct TestingState {
    request: TestingRequest,
    limits: super::limits::TestingLimits,
    iterations: usize,
    tool_calls: usize,
    model_calls: usize,
    /// Whether the loop has switched from evidence gathering to the reserved
    /// final synthesis call.
    synthesis_attempted: bool,
    /// Whether the final prose synthesis was produced.
    synthesis_complete: bool,
    observations: Vec<TestObservation>,
    commands_run: Vec<TestCommandResult>,
    failures: Vec<TestFailure>,
    files_inspected: Vec<PathBuf>,
    final_answer: Option<String>,
    error: Option<String>,
    /// Extra limitations accumulated during the session (e.g. a detected
    /// tracked-tree mutation).
    limitations: Vec<String>,
    git_before: Option<GitStateSnapshot>,
    git_after: Option<GitStateSnapshot>,
}

impl TestingState {
    fn new(request: TestingRequest, limits: super::limits::TestingLimits) -> Self {
        TestingState {
            request,
            limits,
            iterations: 0,
            tool_calls: 0,
            model_calls: 0,
            synthesis_attempted: false,
            synthesis_complete: false,
            observations: Vec::new(),
            commands_run: Vec::new(),
            failures: Vec::new(),
            files_inspected: Vec::new(),
            final_answer: None,
            error: None,
            limitations: Vec::new(),
            git_before: None,
            git_after: None,
        }
    }

    /// Whether the git tree was left unchanged (vacuously true when either
    /// snapshot is absent).
    fn git_tree_unchanged(&self) -> bool {
        match (&self.git_before, &self.git_after) {
            (Some(before), Some(after)) => {
                before.status == after.status && before.clean == after.clean
            }
            _ => true,
        }
    }

    /// Record one authoritative command result and derive its failure.
    fn observe_command(&mut self, record: TestCommandResult) {
        self.commands_run.push(record.clone());
        self.observations.push(TestObservation {
            name: "run_command".to_string(),
            arguments: record.command.clone(),
            result: truncate_chars(&record.render(), self.limits.max_command_output_chars),
            success: record.success,
        });
        if !record.success {
            self.failures.push(TestFailure {
                kind: TestFailureKind::from_command(&record.command, &record),
                command: record.command.clone(),
                exit_code: record.exit_code,
                output: truncate_chars(&record.output, 1000),
            });
        }
    }

    /// Record one read-only tool observation and track inspected files.
    fn observe_tool(&mut self, observation: TestObservation) {
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

    /// Whether the session has gathered any real evidence worth synthesizing.
    fn has_evidence(&self) -> bool {
        self.tool_calls > 0
    }

    /// Compile the testing prompt for the next model call.
    fn build_prompt(&self) -> String {
        let grounding = &self.request.grounding;
        let mut prompt = String::new();
        prompt.push_str(
            "You are CodeBro's autonomous Testing subagent. You execute BOUNDED validation commands and observe authoritative exit codes.\n\n",
        );
        prompt.push_str(&format!("OBJECTIVE:\n{}\n\n", self.request.task));

        prompt.push_str("GROUNDED CONTEXT (initial knowledge):\n");
        prompt.push_str(&format!(
            "Project: {} ({})\n",
            grounding.project_name, grounding.project_language
        ));
        prompt.push_str(&format!(
            "Workspace root: {}\n",
            self.request.workspace_root.display()
        ));
        if !grounding.relevant_files.is_empty() {
            prompt.push_str(&format!(
                "Relevant files: {}\n",
                grounding.relevant_files.join(", ")
            ));
        }
        if !grounding.build_info.is_empty() {
            prompt.push_str(&format!("Build info: {}\n", grounding.build_info));
        }

        prompt.push_str("\nAVAILABLE TOOLS:\n");
        for tool in self.request.limits.describe_tools() {
            prompt.push_str(&format!("- {}\n", tool));
        }
        prompt.push_str(&format!(
            "Permitted validation commands: {}\n",
            self.tooling_policy_description()
        ));

        prompt.push_str("\nPREVIOUS COMMAND RESULTS (authoritative exit codes):\n");
        if self.commands_run.is_empty() {
            prompt.push_str("(none yet)\n");
        } else {
            let rendered =
                render_commands_bounded(&self.commands_run, self.limits.max_output_bytes);
            prompt.push_str(&rendered);
        }

        let tool_observations: Vec<&TestObservation> = self
            .observations
            .iter()
            .filter(|o| o.name != "run_command")
            .collect();
        prompt.push_str("\nPREVIOUS TOOL OBSERVATIONS:\n");
        if tool_observations.is_empty() {
            prompt.push_str("(none yet)\n");
        } else {
            for (i, observation) in tool_observations.iter().enumerate() {
                prompt.push_str(&format!(
                    "  {}. {} {} → {}\n",
                    i + 1,
                    observation.name,
                    observation.arguments,
                    observation.result
                ));
            }
        }

        prompt.push_str(&format!(
            "\nINSTRUCTIONS:\n1. Decide the appropriate validation command for the OBJECTIVE. For a Rust project prefer: cargo check → cargo test → a targeted test if needed. Start with the cheapest check that validates the objective.\n2. The exit code is authoritative: exit 0 = success, non-zero = failure. Do NOT reinterpret the output text — even if the output says 'passed', a non-zero exit code is a failure.\n3. Run only a small number of commands. You have a bounded budget of {} more evidence-gathering call(s).\n4. Use read_file / list_files only when you must inspect specific files; do not scan the whole repository.\n5. NEVER run commands that modify source or git state — they are denied by the Testing policy. Running them wastes budget.\n6. Once you have validated the objective, STOP running commands and produce the final testing report on your next turn.\n\nTESTING STEP {}:\n",
            self.limits.evidence_model_budget().saturating_sub(self.model_calls).max(1),
            self.iterations + 1
        ));
        prompt
    }

    /// The permitted command surface described for the model.
    fn tooling_policy_description(&self) -> String {
        "read-only build/test/lint/format and git-inspect commands permitted by the Testing command policy".to_string()
    }

    /// Compile the reserved final-synthesis prompt. The model sees the full
    /// evidence trail (commands + authoritative exit codes) and must produce
    /// the final testing report WITHOUT any further tool calls.
    fn build_synthesis_prompt(&self) -> String {
        let grounding = &self.request.grounding;
        let mut prompt = String::new();
        prompt.push_str("You are CodeBro's autonomous Testing subagent final synthesis step.\n\n");
        prompt.push_str(&format!("OBJECTIVE:\n{}\n\n", self.request.task));

        prompt.push_str("GROUNDED CONTEXT (initial knowledge):\n");
        prompt.push_str(&format!(
            "Project: {} ({})\n",
            grounding.project_name, grounding.project_language
        ));
        if !grounding.relevant_files.is_empty() {
            prompt.push_str(&format!(
                "Relevant files: {}\n",
                grounding.relevant_files.join(", ")
            ));
        }

        prompt.push_str("\nEVIDENCE — COMMAND RESULTS (authoritative exit codes):\n");
        if self.commands_run.is_empty() {
            prompt.push_str("(no commands were executed)\n");
        } else {
            let rendered =
                render_commands_bounded(&self.commands_run, self.limits.max_output_bytes);
            prompt.push_str(&rendered);
        }

        let tool_observations: Vec<&TestObservation> = self
            .observations
            .iter()
            .filter(|o| o.name != "run_command")
            .collect();
        if !tool_observations.is_empty() {
            prompt.push_str("\nEVIDENCE — TOOL OBSERVATIONS:\n");
            for (i, observation) in tool_observations.iter().enumerate() {
                prompt.push_str(&format!(
                    "  {}. {} {} → {}\n",
                    i + 1,
                    observation.name,
                    observation.arguments,
                    observation.result
                ));
            }
        }

        prompt.push_str(
            "\nINSTRUCTIONS:\n1. Synthesize the evidence above into a concise final testing report that answers the OBJECTIVE.\n2. Do NOT call any tools. This is the final synthesis step; the command budget is exhausted.\n3. Report the exact exit codes and failures. Never claim a command passed if its exit code was non-zero — the exit code is authoritative over the output text.\n4. If the evidence does not answer the objective, say so explicitly rather than inventing results.\n\nFINAL TESTING REPORT:\n",
        );
        prompt
    }

    /// Build the final result.
    fn build_result(&self, termination: TestingTermination, duration_ms: u64) -> TestingResult {
        let summary = self
            .final_answer
            .clone()
            .unwrap_or_else(|| self.default_summary(termination));
        let findings = self.extract_findings();
        let limitations = self.build_limitations(termination);
        let output_size = self.estimate_output();

        TestingResult {
            summary,
            findings,
            commands_run: self.commands_run.clone(),
            files_inspected: self.files_inspected.clone(),
            failures: self.failures.clone(),
            tool_calls: self.tool_calls,
            iterations: self.iterations,
            model_calls: self.model_calls,
            termination,
            synthesis_complete: self.synthesis_complete,
            observations: self.observations.clone(),
            limitations,
            duration_ms,
            output_size,
            provider: String::new(),
            model: String::new(),
            git_before: self.git_before.clone(),
            git_after: self.git_after.clone(),
        }
    }

    /// A deterministic summary when the model never produced a final answer.
    fn default_summary(&self, termination: TestingTermination) -> String {
        format!(
            "Testing terminated with status '{}' after {} iteration(s), {} tool call(s), {} command(s) run, {} failure(s).",
            termination,
            self.iterations,
            self.tool_calls,
            self.commands_run.len(),
            self.failures.len()
        )
    }

    /// Extract findings from the final answer, anchored to the evidence trail.
    fn extract_findings(&self) -> Vec<TestFinding> {
        let mut findings = Vec::new();
        let answer = self.final_answer.clone().unwrap_or_default();
        for line in answer.lines().take(20) {
            let line = line.trim().trim_start_matches(['-', '*', '#', ' ']);
            if line.is_empty() || line.contains("final testing report") {
                continue;
            }
            findings.push(TestFinding {
                statement: line.chars().take(300).collect(),
                kind: None,
                evidence: self
                    .commands_run
                    .last()
                    .map(|c| c.render())
                    .unwrap_or_default(),
            });
        }
        findings
    }

    /// Explicit limitations recorded with the result.
    fn build_limitations(&self, termination: TestingTermination) -> Vec<String> {
        let mut limitations = self.limitations.clone();
        if let Some(error) = &self.error {
            limitations.push(error.clone());
        }
        if !self.git_tree_unchanged() {
            limitations.push("git tracked state changed during testing".to_string());
        }
        match termination {
            TestingTermination::Completed => {}
            TestingTermination::IterationLimit => {
                limitations.push("iteration limit reached".to_string());
            }
            TestingTermination::ToolLimit => {
                limitations.push("tool-call limit reached".to_string());
            }
            TestingTermination::ModelLimit => {
                limitations.push("model-call limit reached".to_string());
            }
            TestingTermination::Timeout => {
                limitations.push("testing timeout reached".to_string());
            }
            TestingTermination::Cancelled => {
                limitations.push("testing cancelled".to_string());
            }
            TestingTermination::Error => {}
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
}

/// Render command results into a prompt, bounded by a total character budget.
/// Command output is the model's observation surface; the authoritative facts
/// (exit code, success) are always present for every command, while long
/// outputs are truncated and later commands are dropped once the budget is
/// exhausted.
fn render_commands_bounded(commands: &[TestCommandResult], max_chars: usize) -> String {
    let mut out = String::new();
    let mut budget = max_chars;
    for (i, command) in commands.iter().enumerate() {
        let mut rendered = format!(
            "  {}. $ {}\n     exit_code: {} | success: {} | duration_ms: {}",
            i + 1,
            command.command,
            command.exit_code,
            command.success,
            command.duration_ms
        );
        if command.denied {
            rendered.push_str(&format!(
                " | denied: {}",
                command.denied_reason.as_deref().unwrap_or("")
            ));
        } else if command.timeout {
            rendered.push_str(" | timed_out: true");
        } else if command.cancelled {
            rendered.push_str(" | cancelled: true");
        }
        if !command.output.trim().is_empty() {
            let tail = truncate_chars(&command.output, 1500);
            rendered.push_str(&format!("\n     output:\n{}", indent(&tail, "     ")));
        }
        rendered.push('\n');
        let len = rendered.chars().count();
        if len > budget && !out.is_empty() {
            out.push_str(&format!(
                "  … ({} more command result(s) omitted — output budget exhausted)\n",
                commands.len() - i
            ));
            break;
        }
        budget = budget.saturating_sub(len);
        out.push_str(&rendered);
    }
    out
}

fn indent(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|l| format!("{prefix}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
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
    fn test_render_commands_bounded_drops_late_commands() {
        let mk = |command: &str, exit: i32| TestCommandResult {
            command: command.to_string(),
            working_directory: "/r".to_string(),
            exit_code: exit,
            success: exit == 0,
            duration_ms: 10,
            output: "x".repeat(150),
            timeout: false,
            cancelled: false,
            denied: false,
            denied_reason: None,
        };
        let commands = vec![
            mk("cargo check", 0),
            mk("cargo test", 101),
            mk("cargo clippy", 101),
        ];
        let rendered = render_commands_bounded(&commands, 600);
        // Every command's authoritative line is present or explicitly omitted.
        assert!(rendered.contains("cargo check"));
        assert!(rendered.contains("exit_code: 0"));
        assert!(rendered.contains("cargo test"));
        assert!(rendered.contains("exit_code: 101"));
        assert!(
            rendered.contains("omitted"),
            "budget exhaustion must be explicit"
        );
    }
}
