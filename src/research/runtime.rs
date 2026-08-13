//! The autonomous Research subagent execution loop (Sprint 30C).
//!
//! This is a real executor, not a template generator. It receives an
//! objective, decides which read-only tool to call, executes it through the
//! restricted registry, observes the result, decides the next action, and
//! iterates until it produces a bounded, evidence-backed `ResearchResult`.
//!
//! ```text
//! ResearchRequest + GroundedContext
//!      ↓
//! ResearchSubagent loop
//!      ├── route provider (IntelligentProviderRouter)
//!      ├── stream via the shared canonical primitive
//!      ├── structured / text tool-call parsing
//!      ├── restricted read-only tool registry
//!      └── observation → next decision
//!      ↓
//! ResearchResult
//! ```

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

use super::contract::{
    truncate_chars, ResearchFinding, ResearchRequest, ResearchResult, ResearchTermination,
    ToolObservation,
};
use super::permissions::ResearchTooling;

/// The bounded research execution runtime.
pub struct ResearchSubagent {
    provider_runtime: ProviderRuntime,
    router: IntelligentProviderRouter,
    io_providers: HashMap<ProviderId, Arc<dyn crate::providers::Provider>>,
    tooling: ResearchTooling,
}

impl ResearchSubagent {
    /// Build a research subagent over the caller's shared provider state and
    /// a restricted read-only tool registry. All components are reused from
    /// the canonical runtime — nothing is re-implemented.
    pub fn new(
        provider_runtime: ProviderRuntime,
        router: IntelligentProviderRouter,
        io_providers: HashMap<ProviderId, Arc<dyn crate::providers::Provider>>,
        tooling: ResearchTooling,
    ) -> Self {
        ResearchSubagent {
            provider_runtime,
            router,
            io_providers,
            tooling,
        }
    }

    /// Run one bounded research session.
    pub async fn run(
        &mut self,
        request: ResearchRequest,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
        cancel: Option<CancellationToken>,
    ) -> ResearchResult {
        let started = Instant::now();
        let limits = request.limits.clone();
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(limits.timeout_ms);

        emit(AgentEvent::Log {
            level: "research".to_string(),
            message: format!("Research started: {}", request.task),
        });
        emit(AgentEvent::AgentStarted {
            agent: "research".to_string(),
            task: request.task.clone(),
        });
        emit(AgentEvent::AgentStatusChanged {
            agent: "research".to_string(),
            status: AgentStatus::Searching,
        });

        let mut state = ResearchState::new(request, limits);
        let mut total_tool_calls = 0usize;

        // The research loop shares the main loop's guards: cancellation,
        // deadline, model-call budget and iteration budget.
        loop {
            // 1. Cancellation.
            if let Some(token) = &cancel {
                if token.is_cancelled() {
                    return self.finish(state, ResearchTermination::Cancelled, started, emit);
                }
            }
            // 2. Deadline.
            if tokio::time::Instant::now() >= deadline {
                return self.finish(state, ResearchTermination::Timeout, started, emit);
            }
            // 3. Model-call budget.
            if state.model_calls >= state.limits.max_model_calls {
                return self.finish(state, ResearchTermination::ModelLimit, started, emit);
            }
            // 4. Iteration budget.
            if state.iterations >= state.limits.max_iterations {
                return self.finish(state, ResearchTermination::IterationLimit, started, emit);
            }

            // 5. Determine the phase: evidence gathering vs final synthesis.
            //    The loop reserves one model call for synthesis so a model that
            //    keeps exploring can never starve the final report. When the
            //    evidence budget is spent (and real evidence was gathered), the
            //    next call is forced to synthesize the findings.
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

                    // Classify the complete response into one of the three
                    // loop states (final text / usable tool calls / neither).
                    // A valid text-only report completes the research
                    // immediately; an empty or malformed response terminates
                    // as a bounded error; only genuine usable tool calls
                    // continue gathering evidence.
                    let calls: Vec<ToolCall> = match execution::classify_response(&full, structured)
                    {
                        execution::ResponseDisposition::Execute(calls) => calls,
                        execution::ResponseDisposition::Final(text) => {
                            state.final_answer = Some(text);
                            state.synthesis_complete = true;
                            let model = provider_model;
                            return self
                                .finish(state, ResearchTermination::Completed, started, emit)
                                .with_provider(provider_id.as_str().to_string(), model);
                        }
                        execution::ResponseDisposition::Empty(msg) => {
                            return self.finish_error(state, msg, started, emit);
                        }
                    };

                    // The reserved synthesis call must not keep gathering
                    // evidence: the model-call budget reserved for it is spent.
                    // Terminate honestly — the structured evidence gathered so
                    // far is preserved and no summary is fabricated.
                    if state.synthesis_attempted {
                        return self.finish(state, ResearchTermination::ModelLimit, started, emit);
                    }

                    // 9. Tool-call budget.
                    if total_tool_calls + calls.len() > state.limits.max_tool_calls {
                        return self.finish(state, ResearchTermination::ToolLimit, started, emit);
                    }

                    for call in &calls {
                        total_tool_calls += 1;
                        state.tool_calls += 1;
                        emit(AgentEvent::ToolStarted {
                            tool: call.name.clone(),
                            args: crate::tools::shell::redact_secrets_public(&call.arguments),
                        });
                        emit(AgentEvent::Log {
                            level: "research".to_string(),
                            message: format!(
                                "Research tool call {}: {}",
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
                            level: "research".to_string(),
                            message: format!(
                                "Research tool result {}: success={}",
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
        state: ResearchState,
        termination: ResearchTermination,
        started: Instant,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> ResearchResult {
        if termination.is_completed() {
            emit(AgentEvent::AgentCompleted {
                agent: "research".to_string(),
                duration_ms: started.elapsed().as_millis() as u64,
            });
        } else {
            emit(AgentEvent::Log {
                level: "research".to_string(),
                message: format!("Research terminated: {}", termination),
            });
        }
        state.build_result(termination, started.elapsed().as_millis() as u64)
    }

    /// Assemble an error result for a session interrupted by a failure.
    fn finish_error(
        &self,
        mut state: ResearchState,
        error: String,
        started: Instant,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> ResearchResult {
        state.error = Some(error.clone());
        emit(AgentEvent::AgentFailed {
            agent: "research".to_string(),
            error: error.clone(),
        });
        emit(AgentEvent::Log {
            level: "research".to_string(),
            message: format!("Research failed: {}", error),
        });
        state.build_result(
            ResearchTermination::Error,
            started.elapsed().as_millis() as u64,
        )
    }
}

/// Accumulated research session state.
struct ResearchState {
    request: ResearchRequest,
    limits: super::limits::ResearchLimits,
    iterations: usize,
    tool_calls: usize,
    model_calls: usize,
    /// Whether the loop has switched from evidence gathering to the reserved
    /// final synthesis call.
    synthesis_attempted: bool,
    /// Whether the final prose synthesis was produced.
    synthesis_complete: bool,
    observations: Vec<ToolObservation>,
    files_inspected: Vec<PathBuf>,
    symbols_found: Vec<String>,
    final_answer: Option<String>,
    error: Option<String>,
}

impl ResearchState {
    fn new(request: ResearchRequest, limits: super::limits::ResearchLimits) -> Self {
        let mut state = ResearchState {
            request,
            limits,
            iterations: 0,
            tool_calls: 0,
            model_calls: 0,
            synthesis_attempted: false,
            synthesis_complete: false,
            observations: Vec::new(),
            files_inspected: Vec::new(),
            symbols_found: Vec::new(),
            final_answer: None,
            error: None,
        };
        // Seed symbols from the grounded context (initial knowledge).
        let grounded_symbols = state.request.grounding.related_symbols.clone();
        state.add_symbols(&grounded_symbols);
        state
    }

    /// Record one real tool observation and extract evidence. The full tool
    /// result is used for symbol extraction (before truncation) so symbols
    /// are recovered even from large files.
    fn observe(&mut self, observation: ToolObservation, full_result: &str) {
        // Track inspected files for path-based tools.
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
        // Extract symbols from file contents that were actually read. The cap
        // is per-file; large files (e.g. the canonical runtime) define many
        // functions and the key entry points appear deep in the file.
        if observation.name == "read_file" {
            self.add_symbols(&extract_symbols(full_result, MAX_SYMBOLS_PER_FILE));
        }
        self.observations.push(observation);
    }

    fn add_file(&mut self, path: PathBuf) {
        let path = self.relativize(path);
        if !self.files_inspected.contains(&path) {
            self.files_inspected.push(path);
        }
    }

    fn add_symbols(&mut self, symbols: &[String]) {
        for symbol in symbols {
            if symbol.is_empty() {
                continue;
            }
            if !self.symbols_found.contains(symbol) {
                self.symbols_found.push(symbol.clone());
            }
        }
        // Keep the overall evidence trail bounded.
        self.symbols_found.truncate(MAX_SYMBOLS_TOTAL);
    }

    /// Whether the session has gathered any real tool evidence worth
    /// synthesizing. Every executed tool call pushes an observation, so a
    /// single successful tool call is sufficient to trigger the final
    /// synthesis when the evidence budget is exhausted.
    fn has_evidence(&self) -> bool {
        self.tool_calls > 0
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

    /// Compile the research prompt for the next model call.
    fn build_prompt(&self) -> String {
        let grounding = &self.request.grounding;
        let mut prompt = String::new();
        prompt.push_str(
            "You are CodeBro's autonomous Research subagent. You perform READ-ONLY repository research.\n\n",
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
        if !grounding.related_symbols.is_empty() {
            prompt.push_str(&format!(
                "Related symbols: {}\n",
                grounding.related_symbols.join(", ")
            ));
        }
        if !grounding.dependencies.is_empty() {
            prompt.push_str(&format!(
                "Dependencies: {}\n",
                grounding.dependencies.join(", ")
            ));
        }
        if !grounding.build_info.is_empty() {
            prompt.push_str(&format!("Build info: {}\n", grounding.build_info));
        }

        prompt.push_str("\nAVAILABLE TOOLS (read-only):\n");
        for tool in self.request.limits.describe_tools() {
            prompt.push_str(&format!("- {}\n", tool));
        }

        prompt.push_str("\nPREVIOUS TOOL OBSERVATIONS:\n");
        if self.observations.is_empty() {
            prompt.push_str("(none yet)\n");
        } else {
            for (i, observation) in self.observations.iter().enumerate() {
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
            "\nINSTRUCTIONS:\n1. Use the available tools to inspect the repository and gather evidence. Prefer a small number of targeted reads over exhaustive listing.\n2. You MUST call at least one tool before producing the final report unless the grounded context already answers the objective.\n3. You have a bounded evidence budget: only {} evidence-gathering call(s) remain before the final synthesis. Gather only the evidence you actually need.\n4. Once you have inspected the relevant files, STOP calling tools and produce the final research report. The report must be evidence-backed and cite the exact file paths and function names you inspected.\n5. Produce the final report on your next turn once you have inspected the relevant files — do not keep exploring.\n\nRESEARCH STEP {}:\n",
            self.limits.evidence_model_budget().saturating_sub(self.model_calls).max(1),
            self.iterations + 1
        ));
        prompt
    }

    /// Compile the reserved final-synthesis prompt. The model sees the full
    /// evidence trail gathered so far and must produce the final research
    /// report WITHOUT any further tool calls. This is the bounded completion
    /// step of the research loop.
    fn build_synthesis_prompt(&self) -> String {
        let grounding = &self.request.grounding;
        let mut prompt = String::new();
        prompt.push_str("You are CodeBro's autonomous Research subagent final synthesis step.\n\n");
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
        if !grounding.related_symbols.is_empty() {
            prompt.push_str(&format!(
                "Related symbols: {}\n",
                grounding.related_symbols.join(", ")
            ));
        }
        if !grounding.dependencies.is_empty() {
            prompt.push_str(&format!(
                "Dependencies: {}\n",
                grounding.dependencies.join(", ")
            ));
        }

        prompt.push_str("\nEVIDENCE GATHERED (tool observations):\n");
        if self.observations.is_empty() {
            prompt.push_str("(no tool observations — synthesize from grounded context only)\n");
        } else {
            for (i, observation) in self.observations.iter().enumerate() {
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
            "\nINSTRUCTIONS:\n1. Synthesize the evidence above into a concise final research report that answers the OBJECTIVE.\n2. Do NOT call any tools. This is the final synthesis step; the evidence budget is exhausted.\n3. Base every claim ONLY on the evidence above or the grounded context. If the evidence does not answer the objective, say so explicitly rather than inventing details.\n4. Cite the exact file paths and function names from the evidence.\n\nFINAL RESEARCH REPORT:\n",
        );
        prompt
    }

    /// Extract structured findings and build the final result.
    fn build_result(&self, termination: ResearchTermination, duration_ms: u64) -> ResearchResult {
        let summary = self
            .final_answer
            .clone()
            .unwrap_or_else(|| self.default_summary(termination));
        let findings = self.extract_findings();
        let limitations = self.build_limitations(termination);
        let output_size = self.estimate_output();

        ResearchResult {
            summary,
            findings,
            files_inspected: self.files_inspected.clone(),
            symbols_found: self.symbols_found.clone(),
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

    /// A deterministic summary when the model never produced a final answer.
    fn default_summary(&self, termination: ResearchTermination) -> String {
        format!(
            "Research terminated with status '{}' after {} iteration(s), {} tool call(s), inspecting {} file(s).",
            termination,
            self.iterations,
            self.tool_calls,
            self.files_inspected.len()
        )
    }

    /// Extract findings from the final answer, anchored to the evidence trail.
    fn extract_findings(&self) -> Vec<ResearchFinding> {
        let mut findings = Vec::new();
        let answer = self.final_answer.clone().unwrap_or_default();
        for line in answer.lines() {
            let line = line.trim().trim_start_matches(['-', '*', '#', ' ']);
            if line.is_empty() {
                continue;
            }
            // Skip instructional boilerplate.
            if line.contains("final research report")
                || line.starts_with("RESEARCH STEP")
                || line.starts_with("You are CodeBro")
            {
                continue;
            }
            let evidence = self
                .observations
                .last()
                .map(|o| format!("{} {} → {}", o.name, o.arguments, o.result))
                .unwrap_or_default();
            let anchor = self.observations.iter().find(|o| o.name == "read_file");
            findings.push(ResearchFinding {
                statement: line.chars().take(300).collect(),
                file: anchor.and_then(|o| parse_arg_path(&o.arguments).map(PathBuf::from)),
                symbol: None,
                evidence: truncate_chars(&evidence, 300),
            });
        }
        findings.truncate(12);
        findings
    }

    /// Explicit limitations recorded with the result.
    fn build_limitations(&self, termination: ResearchTermination) -> Vec<String> {
        let mut limitations = Vec::new();
        if let Some(error) = &self.error {
            limitations.push(error.clone());
        }
        match termination {
            ResearchTermination::Completed => {}
            ResearchTermination::IterationLimit => {
                limitations.push("iteration limit reached".to_string());
            }
            ResearchTermination::ToolLimit => {
                limitations.push("tool-call limit reached".to_string());
            }
            ResearchTermination::ModelLimit => {
                limitations.push("model-call limit reached".to_string());
            }
            ResearchTermination::Timeout => {
                limitations.push("research timeout reached".to_string());
            }
            ResearchTermination::Cancelled => {
                limitations.push("research cancelled".to_string());
            }
            ResearchTermination::Error => {}
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

impl ResearchResult {
    /// Record the provider/model that executed the research.
    pub fn with_provider(mut self, provider: String, model: String) -> Self {
        self.provider = provider;
        self.model = model;
        self
    }
}

/// Parse the `path` argument from a tool-call JSON argument string.
fn parse_arg_path(arguments: &str) -> Option<String> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Handle both JSON-wrapped and raw path arguments.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(path) = value.get("path").and_then(|v| v.as_str()) {
            return Some(path.to_string());
        }
    }
    // Raw path (may be quoted).
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

/// Deterministically extract likely symbol names from source text (bounded).
/// Key entry points (e.g. the canonical runtime's `run_execution_loop`,
/// `execute_tool`) are defined deep in large files, so the per-file cap is set
/// high enough to surface them while the overall result stays bounded.
const MAX_SYMBOLS_PER_FILE: usize = 40;
const MAX_SYMBOLS_TOTAL: usize = 80;

fn extract_symbols(content: &str, max: usize) -> Vec<String> {
    let mut symbols = Vec::new();
    let mut lines = content.lines();
    let mut prev = String::new();
    let mut line = lines.next();
    while let Some(current) = line {
        let current = current.trim();
        // `fn name`, `pub fn name`, `struct`, `enum`, `trait`, `impl`.
        let lowered = current.to_lowercase();
        for keyword in ["fn ", "struct ", "enum ", "trait ", "impl "] {
            if let Some(idx) = current.find(keyword) {
                let rest = &current[idx + keyword.len()..];
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() && !symbols.contains(&name) {
                    symbols.push(name);
                }
                break;
            }
        }
        if current.starts_with("pub ") && prev.contains("pub ") {
            // no-op: avoid false positives on repeated modifiers
        }
        prev = current.to_string();
        line = lines.next();
        if symbols.len() >= max {
            break;
        }
    }
    symbols
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
    fn test_extract_symbols_finds_functions() {
        let content = "pub fn parse_tool_calls() {}\nstruct Foo {}\nenum Bar {}\nfn run_execution_loop() {}\n";
        let symbols = extract_symbols(content, 12);
        assert!(symbols.contains(&"parse_tool_calls".to_string()));
        assert!(symbols.contains(&"Foo".to_string()));
        assert!(symbols.contains(&"Bar".to_string()));
        assert!(symbols.contains(&"run_execution_loop".to_string()));
    }

    #[test]
    fn test_extract_symbols_bounded() {
        let content = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n";
        let symbols = extract_symbols(content, 3);
        assert_eq!(symbols.len(), 3);
    }
}
