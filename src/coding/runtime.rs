//! The autonomous Coding subagent execution loop (Sprint 30F).
//!
//! This is a real executor, not a template generator. It receives the user
//! objective plus the evidence of Research (what exists), Testing (what
//! works) and — crucially — the REAL `PlanningResult` (what must change and
//! how to validate), decides which targeted change to apply, routes every
//! mutation through the [`ChangeEngine`] (which routes existing-file writes
//! through [`ChangePlan`](crate::tools::ChangePlan)/[`PatchEngine`](crate::tools::PatchEngine)
//! and file creation through the engine's documented controlled creation
//! seam), verifies through the policy-checked Testing surface, iterates, and
//! finishes with a bounded, auditable `CodingResult`.
//!
//! ```text
//! CodingRequest + GroundedContext + ResearchResult + TestingResult + PlanningResult
//!      ↓
//! CodingSubagent loop
//!      ├── route provider (IntelligentProviderRouter)
//!      ├── stream via the shared canonical primitive (execution::stream_once)
//!      ├── structured / text tool-call parsing
//!      ├── propose_change → ChangeEngine (boundary / plan / ambiguity / stale)
//!      ├── verify → TestingTooling (authoritative exit code, policy-checked)
//!      ├── bounded revision on explicit verify failure
//!      ├── reserved final synthesis → completion gate auto-verifies
//!      │    (no plan validation commands → VerificationUnavailable — never a
//!      │    fabricated verification)
//!      ↓
//! CodingResult
//! ```
//!
//! Safety contract:
//! - Coding NEVER calls raw `fs::write`/`remove_file` on source files; every
//!   mutation is a `ChangePlan.apply` behind the engine boundary (file
//!   creation stays inside the engine as the sole controlled create seam).
//! - A terminal failure (VerificationFailed / Error) rolls back ONLY the
//!   session's own changes, in reverse order; created files are removed only
//!   while their content is still exactly what the session wrote.
//! - Pre-existing user changes are preserved: proposals require a unique
//!   `old` match against current content, and stale files are never clobbered.
//! - Git history is NEVER mutated: no commits, no checkouts.
//! - The machine owns success: verification `success` comes exclusively from
//!   the process exit code, never from output prose, and a session that
//!   applied changes without any validation commands terminates as
//!   `VerificationUnavailable` — unverified, never completed-as-verified.

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
    AppliedChange, CodingObservation, CodingRequest, CodingResult, CodingTermination,
    VerificationRecord, VerificationSource,
};
use super::limits::CodingLimits;
use super::permissions::{self, parse_proposal_args, CodingTooling};

/// The bounded coding execution runtime.
pub struct CodingSubagent {
    provider_runtime: ProviderRuntime,
    router: IntelligentProviderRouter,
    io_providers: HashMap<ProviderId, Arc<dyn crate::providers::Provider>>,
    tooling: CodingTooling,
}

impl CodingSubagent {
    /// Build a coding subagent over the caller's shared provider state and a
    /// restricted, engine-bound mutation tooling. All components are reused
    /// from the canonical runtime — nothing is re-implemented.
    pub fn new(
        provider_runtime: ProviderRuntime,
        router: IntelligentProviderRouter,
        io_providers: HashMap<ProviderId, Arc<dyn crate::providers::Provider>>,
        tooling: CodingTooling,
    ) -> Self {
        CodingSubagent {
            provider_runtime,
            router,
            io_providers,
            tooling,
        }
    }

    /// Run one bounded coding session.
    pub async fn run(
        &mut self,
        request: CodingRequest,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
        cancel: Option<CancellationToken>,
    ) -> CodingResult {
        let started = Instant::now();
        let limits = request.limits.clone();
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(limits.timeout_ms);

        emit(AgentEvent::Log {
            level: "coding".to_string(),
            message: format!("Coding started: {}", request.task),
        });
        emit(AgentEvent::AgentStarted {
            agent: "coding".to_string(),
            task: request.task.clone(),
        });
        emit(AgentEvent::AgentStatusChanged {
            agent: "coding".to_string(),
            status: AgentStatus::Executing,
        });

        let mut state = CodingState::new(request, limits);
        let mut total_tool_calls = 0usize;

        // Baseline snapshot: the tracked tree BEFORE this session mutated it.
        state.git_before = Some(self.tooling.check_git_state());

        loop {
            // 1. Cancellation.
            if let Some(token) = &cancel {
                if token.is_cancelled() {
                    return self.finish(state, CodingTermination::Cancelled, started, emit);
                }
            }
            // 2. Deadline.
            if tokio::time::Instant::now() >= deadline {
                return self.finish(state, CodingTermination::Timeout, started, emit);
            }
            // 3. Model-call budget.
            if state.model_calls >= state.limits.max_model_calls {
                return self.finish(state, CodingTermination::ModelLimit, started, emit);
            }
            // 4. Iteration budget.
            if state.iterations >= state.limits.max_iterations {
                return self.finish(state, CodingTermination::IterationLimit, started, emit);
            }

            // 5. Determine the phase: execution vs final synthesis. The loop
            //    reserves one model call for the final report, so a model that
            //    keeps mutating can never starve the synthesis (and the
            //    completion-gate verification).
            if !state.synthesis_attempted
                && state.model_calls >= state.limits.evidence_model_budget()
                && state.has_evidence()
            {
                state.synthesis_attempted = true;
                emit(AgentEvent::Log {
                    level: "coding".to_string(),
                    message: "Coding entering final synthesis phase".to_string(),
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
                level: "coding".to_string(),
                message: format!("Coding model call {}", state.model_calls + 1),
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

                    // No tool call → the model produced its final report. The
                    // completion gate auto-verifies any change that has no
                    // explicit successful verification yet (using the plan's
                    // validation commands, through the authoritative Testing
                    // surface). A gate failure is immediate and terminal; a
                    // session with applied changes but NO plan validation
                    // commands terminates as VerificationUnavailable and is
                    // never reported as machine-verified.
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

                    // The reserved synthesis call must not keep mutating:
                    // terminate honestly — the applied changes and verification
                    // records are preserved and no report is fabricated.
                    if state.synthesis_attempted {
                        return self.finish(state, CodingTermination::ModelLimit, started, emit);
                    }

                    // 9. Tool-call budget.
                    if total_tool_calls + calls.len() > state.limits.max_tool_calls {
                        return self.finish(state, CodingTermination::ToolLimit, started, emit);
                    }

                    for call in &calls {
                        total_tool_calls += 1;
                        state.tool_calls += 1;
                        emit(AgentEvent::ToolStarted {
                            tool: call.name.clone(),
                            args: crate::tools::shell::redact_secrets_public(&call.arguments),
                        });
                        emit(AgentEvent::Log {
                            level: "coding".to_string(),
                            message: format!(
                                "Coding tool call {}: {}",
                                state.tool_calls, call.name
                            ),
                        });

                        match call.name.as_str() {
                            "propose_change" => {
                                let (result, success) = self.propose(&mut state, &call.arguments);
                                emit(AgentEvent::Log {
                                    level: "coding".to_string(),
                                    message: format!(
                                        "Coding change result {}: success={}",
                                        state.tool_calls, success
                                    ),
                                });
                                emit(AgentEvent::ToolCompleted {
                                    tool: call.name.clone(),
                                    result: result.clone(),
                                    success,
                                });
                                state.observe(CodingObservation {
                                    name: call.name.clone(),
                                    arguments: call.arguments.clone(),
                                    result,
                                    success,
                                });
                            }
                            "verify" => {
                                let (record, truncated) = self
                                    .verify(&mut state, &call.arguments, cancel.clone())
                                    .await;
                                emit(AgentEvent::Log {
                                    level: "coding".to_string(),
                                    message: format!(
                                        "Coding verification completed: exit={} success={} denied={} timeout={}",
                                        record.exit_code, record.success, record.denied, record.timeout
                                    ),
                                });
                                emit(AgentEvent::ToolCompleted {
                                    tool: call.name.clone(),
                                    result: truncated.clone(),
                                    success: record.success,
                                });
                                state.observe(CodingObservation {
                                    name: call.name.clone(),
                                    arguments: call.arguments.clone(),
                                    result: truncated,
                                    success: record.success,
                                });

                                // Bounded revision: a failed explicit
                                // verification may be retried within the
                                // budget; beyond it the session fails and
                                // rolls back.
                                if !record.success {
                                    state.revisions += 1;
                                    if state.revisions >= state.limits.max_revision_attempts {
                                        return self.finish(
                                            state,
                                            CodingTermination::VerificationFailed,
                                            started,
                                            emit,
                                        );
                                    }
                                    emit(AgentEvent::Log {
                                        level: "coding".to_string(),
                                        message: format!(
                                            "Coding verification failed, revising (attempt {}/{})",
                                            state.revisions, state.limits.max_revision_attempts
                                        ),
                                    });
                                }
                            }
                            _ => {
                                // 10. Read-only inspection tools.
                                let result = self
                                    .tooling
                                    .execute_tool(&call.name, &call.arguments, cancel.clone())
                                    .await;
                                let truncated =
                                    truncate_chars(&result, state.limits.max_tool_result_chars);
                                let success = !result.starts_with("Error:");
                                emit(AgentEvent::Log {
                                    level: "coding".to_string(),
                                    message: format!(
                                        "Coding tool result {}: success={}",
                                        state.tool_calls, success
                                    ),
                                });
                                emit(AgentEvent::ToolCompleted {
                                    tool: call.name.clone(),
                                    result: truncated.clone(),
                                    success,
                                });
                                state.observe(CodingObservation {
                                    name: call.name.clone(),
                                    arguments: call.arguments.clone(),
                                    result: truncated,
                                    success,
                                });
                            }
                        }
                    }
                }
                Err(e) => {
                    return self.finish_error(state, e, started, emit);
                }
            }
        }
    }

    /// Handle a `propose_change` call: parse, prepare against current content
    /// (read-only, boundary/plan/ambiguity/blind-overwrite enforcement),
    /// then apply through the engine. Returns the model-observable result.
    fn propose(&self, state: &mut CodingState, arguments: &str) -> (String, bool) {
        let Some((path, old, new)) = parse_proposal_args(arguments) else {
            return (
                "Error: propose_change expects {\"path\": ..., \"old\": ..., \"new\": ...} (or path|old|new)".to_string(),
                false,
            );
        };
        let prepared = match self.tooling.engine.prepare(&path, &old, &new) {
            Ok(prepared) => prepared,
            Err(e) => {
                return (format!("Error: {e}"), false);
            }
        };
        let applied = match self.tooling.engine.apply(&prepared) {
            Ok(applied) => applied,
            Err(e) => {
                return (format!("Error: {e}"), false);
            }
        };
        let relative = prepared
            .path
            .strip_prefix(&self.tooling.workspace_root)
            .unwrap_or(&prepared.path)
            .to_path_buf();
        state.changes.push(AppliedChange {
            path: relative.clone(),
            created: prepared.created,
            unplanned: prepared.unplanned,
            preview: prepared.preview.clone(),
            backup: prepared.backup.clone(),
            full_new: prepared.full_new.clone(),
            verified: false,
            rolled_back: false,
        });
        if prepared.unplanned {
            state.limitations.push(format!(
                "unplanned change recorded: '{}' is not among the plan's affected files",
                relative.display()
            ));
        }
        let mut result = String::from("Change applied");
        if prepared.unplanned {
            result.push_str(
                " [UNPLANNED: this file is not among the plan's affected files — deviation recorded]",
            );
        }
        result.push_str(&format!(
            " to {}\n{}\n{}",
            relative.display(),
            prepared.preview,
            applied
        ));
        (
            permissions::truncate_and_redact(&result, state.limits.max_tool_result_chars),
            true,
        )
    }

    /// Handle a `verify` call: execute through the policy-checked Testing
    /// surface and capture the authoritative exit code. On success, every
    /// outstanding applied change becomes verified.
    async fn verify(
        &mut self,
        state: &mut CodingState,
        arguments: &str,
        cancel: Option<CancellationToken>,
    ) -> (VerificationRecord, String) {
        let record = self
            .tooling
            .execute_verify(arguments, VerificationSource::Explicit, cancel)
            .await;
        state.verification.push(record.clone());
        if record.success {
            state.mark_all_verified();
        }
        let truncated =
            permissions::truncate_and_redact(&record.render(), state.limits.max_tool_result_chars);
        (record, truncated)
    }

    /// The final-answer completion gate: any applied change without an
    /// explicit successful verification is auto-verified against the plan's
    /// ordered validation commands (authoritative exit codes). A gate failure
    /// is IMMEDIATE and terminal — no revision is offered.
    ///
    /// If changes were applied but the plan carries NO validation commands,
    /// the gate MUST NOT fabricate success: it terminates as
    /// [`CodingTermination::VerificationUnavailable`] with the changes left in
    /// place and honestly marked unverified (`verified == false`).
    async fn completion_gate(
        &mut self,
        state: &mut CodingState,
        cancel: Option<CancellationToken>,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> CodingTermination {
        if !state.has_unverified_changes() {
            return CodingTermination::Completed;
        }
        emit(AgentEvent::Log {
            level: "coding".to_string(),
            message: "Coding completion gate: auto-verifying unverified changes".to_string(),
        });
        let commands = state.request.plan_validation_commands();
        if commands.is_empty() {
            // The plan offers NO validation surface, yet changes were applied.
            // There is nothing authoritative the machine can run, so the
            // session must not claim verification: the changes stay in place,
            // reported honestly as unverified.
            state.limitations.push(
                "no validation commands in the plan: applied changes could not be machine-verified and remain unverified".to_string(),
            );
            return CodingTermination::VerificationUnavailable;
        }
        for command in &commands {
            let record = self
                .tooling
                .execute_verify(command, VerificationSource::CompletionGate, cancel.clone())
                .await;
            state.verification.push(record.clone());
            state.observe(CodingObservation {
                name: "verify".to_string(),
                arguments: command.clone(),
                result: permissions::truncate_and_redact(
                    &record.render(),
                    state.limits.max_tool_result_chars,
                ),
                success: record.success,
            });
            emit(AgentEvent::Log {
                level: "coding".to_string(),
                message: format!(
                    "Coding completion gate verification: exit={} success={}",
                    record.exit_code, record.success
                ),
            });
            if !record.success {
                return CodingTermination::VerificationFailed;
            }
        }
        state.mark_all_verified();
        CodingTermination::Completed
    }

    /// Roll back the session's OWN changes, in reverse application order.
    ///
    /// - Created files are removed ONLY while their content is still exactly
    ///   what the session wrote (a file touched since is left untouched).
    /// - Modified files are restored to their pre-change backup ONLY when the
    ///   session's own change is still present (never clobbering content that
    ///   is not the session's).
    /// - Git history is never touched.
    fn rollback(
        &self,
        state: &mut CodingState,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> Vec<String> {
        let mut log = Vec::new();
        for change in state.changes.iter_mut().rev() {
            if change.rolled_back {
                continue;
            }
            let abs = self.tooling.workspace_root.join(&change.path);
            if change.created {
                match std::fs::read_to_string(&abs) {
                    Ok(current) if current == change.full_new => {
                        if std::fs::remove_file(&abs).is_ok() {
                            change.rolled_back = true;
                            log.push(format!(
                                "rolled back created file {}",
                                change.path.display()
                            ));
                        } else {
                            log.push(format!(
                                "could not remove created file {}",
                                change.path.display()
                            ));
                        }
                    }
                    Ok(_) => {
                        log.push(format!(
                            "created file {} left untouched (content modified since)",
                            change.path.display()
                        ));
                    }
                    Err(e) => {
                        log.push(format!(
                            "created file {} already gone ({e})",
                            change.path.display()
                        ));
                    }
                }
            } else {
                match std::fs::read_to_string(&abs) {
                    Ok(current) if current == change.full_new => {
                        if std::fs::write(&abs, &change.backup).is_ok() {
                            change.rolled_back = true;
                            log.push(format!(
                                "restored original content of {}",
                                change.path.display()
                            ));
                        } else {
                            log.push(format!("could not restore {}", change.path.display()));
                        }
                    }
                    Ok(_) => {
                        log.push(format!(
                            "{} left untouched (content modified since)",
                            change.path.display()
                        ));
                    }
                    Err(e) => {
                        log.push(format!(
                            "{} unreadable during rollback ({e})",
                            change.path.display()
                        ));
                    }
                }
            }
        }
        if log.is_empty() {
            log.push("no session changes to roll back".to_string());
        }
        for entry in &log {
            emit(AgentEvent::Log {
                level: "coding".to_string(),
                message: format!("Coding rollback: {entry}"),
            });
        }
        log
    }

    /// Assemble the final result for a terminating session.
    fn finish(
        &mut self,
        mut state: CodingState,
        termination: CodingTermination,
        started: Instant,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> CodingResult {
        state.git_after = Some(self.tooling.check_git_state());

        // Terminal failures restore the repository to the session's baseline.
        // Bound terminations (tool/model/timeout/cancel) leave the applied
        // changes in place for the caller to inspect.
        if termination.requires_rollback() {
            let rollback_log = self.rollback(&mut state, emit);
            state.limitations.extend(rollback_log);
        }

        if termination.is_completed() {
            emit(AgentEvent::AgentCompleted {
                agent: "coding".to_string(),
                duration_ms: started.elapsed().as_millis() as u64,
            });
        } else {
            emit(AgentEvent::Log {
                level: "coding".to_string(),
                message: format!("Coding terminated: {}", termination),
            });
        }
        state.build_result(termination, started.elapsed().as_millis() as u64)
    }

    /// Assemble an error result for a session interrupted by a failure. The
    /// session's own changes are rolled back (Error is a terminal failure).
    fn finish_error(
        &mut self,
        mut state: CodingState,
        error: String,
        started: Instant,
        emit: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> CodingResult {
        state.error = Some(error.clone());
        emit(AgentEvent::AgentFailed {
            agent: "coding".to_string(),
            error: error.clone(),
        });
        emit(AgentEvent::Log {
            level: "coding".to_string(),
            message: format!("Coding failed: {}", error),
        });
        state.git_after = Some(self.tooling.check_git_state());
        let rollback_log = self.rollback(&mut state, emit);
        state.limitations.extend(rollback_log);
        state.build_result(
            CodingTermination::Error,
            started.elapsed().as_millis() as u64,
        )
    }
}

/// Accumulated coding session state.
struct CodingState {
    request: CodingRequest,
    limits: CodingLimits,
    iterations: usize,
    tool_calls: usize,
    model_calls: usize,
    /// Number of failed explicit verification attempts.
    revisions: usize,
    /// Whether the loop has switched from execution to the reserved final
    /// synthesis call.
    synthesis_attempted: bool,
    /// Whether the final prose synthesis was produced.
    synthesis_complete: bool,
    observations: Vec<CodingObservation>,
    files_inspected: Vec<PathBuf>,
    changes: Vec<AppliedChange>,
    verification: Vec<VerificationRecord>,
    final_answer: Option<String>,
    error: Option<String>,
    /// Extra limitations accumulated during the session (deviations,
    /// rollback log).
    limitations: Vec<String>,
    git_before: Option<crate::testing::GitStateSnapshot>,
    git_after: Option<crate::testing::GitStateSnapshot>,
}

impl CodingState {
    fn new(request: CodingRequest, limits: CodingLimits) -> Self {
        CodingState {
            request,
            limits,
            iterations: 0,
            tool_calls: 0,
            model_calls: 0,
            revisions: 0,
            synthesis_attempted: false,
            synthesis_complete: false,
            observations: Vec::new(),
            files_inspected: Vec::new(),
            changes: Vec::new(),
            verification: Vec::new(),
            final_answer: None,
            error: None,
            limitations: Vec::new(),
            git_before: None,
            git_after: None,
        }
    }

    /// Record one real tool observation and track inspected files.
    fn observe(&mut self, observation: CodingObservation) {
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

    /// Whether any applied change is not yet covered by a successful
    /// verification.
    fn has_unverified_changes(&self) -> bool {
        self.changes.iter().any(|c| !c.verified && !c.rolled_back)
    }

    /// Mark every outstanding change as verified. Call ONLY after an
    /// authoritative machine verification actually ran and succeeded
    /// (exit code 0) — via an explicit `verify` or a completed completion
    /// gate. Never call this to "honor" model prose or when no validation
    /// command could run.
    fn mark_all_verified(&mut self) {
        for change in &mut self.changes {
            if !change.rolled_back {
                change.verified = true;
            }
        }
    }

    /// Whether the session has any real evidence worth synthesizing.
    fn has_evidence(&self) -> bool {
        self.tool_calls > 0 || !self.changes.is_empty() || !self.verification.is_empty()
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

    /// RESEARCH EVIDENCE section (consumed facts, not rediscovery).
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

    /// TESTING EVIDENCE section (authoritative exit codes).
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

    /// IMPLEMENTATION PLAN section — rendered from the REAL structured plan so
    /// the model enforces plan adherence against concrete files.
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
                if !step.target_symbols.is_empty() {
                    out.push_str(&format!("   Symbols: {}\n", step.target_symbols.join(", ")));
                }
                if !step.validation.is_empty() {
                    out.push_str(&format!("   Validate: {}\n", step.validation.join("; ")));
                }
                if !step.risk.is_empty() {
                    out.push_str(&format!("   Risk: {}\n", step.risk));
                }
            }
        }
        if !planning.risks.is_empty() {
            out.push_str("Plan risks:\n");
            for risk in planning.risks.iter().take(6) {
                out.push_str(&format!(
                    "- {} (severity: {})\n",
                    risk.description, risk.severity
                ));
            }
        }
        out
    }

    /// CURRENT CHANGES section: every applied change with its preview and
    /// verification status.
    fn render_changes(&self) -> String {
        if self.changes.is_empty() {
            return "(no changes applied yet)\n".to_string();
        }
        let mut out = String::new();
        for (i, change) in self.changes.iter().enumerate() {
            out.push_str(&format!(
                "  {}. {} [{}]{}\n",
                i + 1,
                change.path.display(),
                change.status(),
                if change.unplanned { " [UNPLANNED]" } else { "" }
            ));
            for line in change.preview.lines().take(12) {
                out.push_str(&format!("     {}\n", line));
            }
        }
        out
    }

    /// VERIFICATION section: authoritative exit-code records.
    fn render_verification(&self) -> String {
        if self.verification.is_empty() {
            return "(no verification commands executed yet)\n".to_string();
        }
        let mut out = String::new();
        for (i, record) in self.verification.iter().enumerate() {
            out.push_str(&format!(
                "  {}. {} (source: {}) → exit_code: {}, success: {}{}{}\n",
                i + 1,
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
        out
    }

    fn render_observations(&self) -> String {
        let read_only: Vec<&CodingObservation> = self
            .observations
            .iter()
            .filter(|o| o.name != "propose_change" && o.name != "verify" && o.name != "run_command")
            .collect();
        if read_only.is_empty() {
            return "(none yet)\n".to_string();
        }
        let mut out = String::new();
        for (i, observation) in read_only.iter().enumerate() {
            out.push_str(&format!(
                "  {}. {} {} → {}\n",
                i + 1,
                observation.name,
                observation.arguments,
                truncate_chars(&observation.result, 400)
            ));
        }
        out
    }

    // =====================================================================
    // Prompts
    // =====================================================================

    /// Compile the coding prompt for the next model call. Evidence sections
    /// stay distinct — provenance matters.
    fn build_prompt(&self) -> String {
        let mut prompt = String::new();
        prompt.push_str(
            "You are CodeBro's autonomous Coding subagent. You implement the plan by applying TARGETED, REVERSIBLE changes to repository files. Until you finish, YOU are the only component allowed to modify files.\n\n",
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

        prompt.push_str("IMPLEMENTATION PLAN (must follow):\n");
        prompt.push_str(&self.render_plan());
        prompt.push('\n');

        prompt.push_str("CURRENT APPLIED CHANGES:\n");
        prompt.push_str(&self.render_changes());
        prompt.push('\n');

        prompt.push_str("CURRENT VERIFICATION (authoritative exit codes):\n");
        prompt.push_str(&self.render_verification());
        prompt.push('\n');

        prompt.push_str("CURRENT READ-ONLY OBSERVATIONS:\n");
        prompt.push_str(&self.render_observations());

        prompt.push_str("\nAVAILABLE TOOLS:\n");
        for tool in self.request.limits.describe_tools() {
            prompt.push_str(&format!("- {}\n", tool));
        }

        prompt.push_str(&format!(
            "\nINSTRUCTIONS:\n1. Follow the IMPLEMENTATION PLAN steps in order. Changes to files outside the plan's affected files are NOT silently applied: each is recorded as an UNPLANNED change and surfaced in the final report{}\n2. propose_change is the ONLY mutation surface and it is a REAL, immediately-applied change: it prepares against the CURRENT file content, and `old` must match that content EXACTLY ONCE. An empty `old` is ONLY valid for creating a NEW file with `new` holding the full content. Ambiguous matches, stale content and blind overwrites are denied.\n3. Inspect files (read_file) before proposing changes so `old` matches the real content.\n4. After applying changes, run verify with the plan's validation command (e.g. cargo check, cargo test). The exit code is authoritative: exit 0 = success, non-zero = failure — do NOT reinterpret the output text.\n5. A failed verify consumes one of your {} revision attempt(s); after that the session fails and ALL your changes are rolled back.\n6. You have a bounded execution budget: {} execution call(s) remain before the final synthesis, and {} total tool calls.\n7. When all changes are applied AND verified, STOP calling tools and produce the final report on your next turn.\n\nCODING STEP {}:\n",
            if self.limits.strict_plan_adherence {
                " — and with strict plan adherence enabled they are DENIED outright."
            } else {
                "."
            },
            self.limits.max_revision_attempts,
            self.limits.evidence_model_budget().saturating_sub(self.model_calls).max(1),
            self.limits.max_tool_calls,
            self.iterations + 1
        ));
        prompt
    }

    /// Compile the reserved final-synthesis prompt. The model sees the full
    /// evidence trail (changes + authoritative exit codes) and must produce
    /// the final coding report WITHOUT any further tool calls.
    fn build_synthesis_prompt(&self) -> String {
        let mut prompt = String::new();
        prompt.push_str(
            "You are CodeBro's autonomous Coding subagent final synthesis step. Produce the FINAL CODING REPORT. No tools are available.\n\n",
        );
        prompt.push_str(&format!("USER OBJECTIVE:\n{}\n\n", self.request.task));

        prompt.push_str("IMPLEMENTATION PLAN (was followed):\n");
        prompt.push_str(&self.render_plan());
        prompt.push('\n');

        prompt.push_str("APPLIED CHANGES:\n");
        prompt.push_str(&self.render_changes());
        prompt.push('\n');

        prompt.push_str("VERIFICATION (authoritative exit codes):\n");
        prompt.push_str(&self.render_verification());

        prompt.push_str(
            "\nINSTRUCTIONS:\n1. Synthesize the evidence above into a concise FINAL CODING REPORT: what was changed, in which files, and the verification outcome for each validation command.\n2. Do NOT call any tools. This is the final synthesis step; the execution budget is exhausted.\n3. Never claim a verification passed if its exit code was non-zero — the exit code is authoritative over the output text.\n4. Report any UNPLANNED changes explicitly (they were recorded, never silent).\n5. If nothing was changed, say so and explain why.\n\nFINAL CODING REPORT FORMAT:\n## Changed files\n<one line per file with a one-line summary>\n\n## Verification\n<per-command: command, exit code, success>\n\n## Deviation from plan\n<none, or explicit unplanned changes>\n\nFINAL CODING REPORT:\n",
        );
        prompt
    }

    // =====================================================================
    // Result assembly
    // =====================================================================

    fn build_result(&self, termination: CodingTermination, duration_ms: u64) -> CodingResult {
        let summary = self
            .final_answer
            .clone()
            .unwrap_or_else(|| self.default_summary(termination));
        let unplanned_changes = self
            .changes
            .iter()
            .filter(|c| c.unplanned)
            .cloned()
            .collect();
        let limitations = self.build_limitations(termination);
        let output_size = self.estimate_output();

        CodingResult {
            summary,
            changes: self.changes.clone(),
            unplanned_changes,
            verification: self.verification.clone(),
            files_inspected: self.files_inspected.clone(),
            tool_calls: self.tool_calls,
            iterations: self.iterations,
            model_calls: self.model_calls,
            revisions: self.revisions,
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

    /// A deterministic summary when the model never produced a final report.
    fn default_summary(&self, termination: CodingTermination) -> String {
        format!(
            "Coding terminated with status '{}' after {} iteration(s), {} tool call(s), {} change(s) applied, {} verification command(s) run, {} revision(s).",
            termination,
            self.iterations,
            self.tool_calls,
            self.changes.len(),
            self.verification.len(),
            self.revisions
        )
    }

    fn build_limitations(&self, termination: CodingTermination) -> Vec<String> {
        let mut limitations = self.limitations.clone();
        if let Some(error) = &self.error {
            limitations.push(error.clone());
        }
        match termination {
            CodingTermination::Completed => {}
            CodingTermination::VerificationUnavailable => {
                // The completion gate records the reason when it emits this
                // termination (the plan carried no validation commands).
            }
            CodingTermination::IterationLimit => {
                limitations.push("iteration limit reached".to_string());
            }
            CodingTermination::ToolLimit => {
                limitations.push("tool-call limit reached".to_string());
            }
            CodingTermination::ModelLimit => {
                limitations.push("model-call limit reached".to_string());
            }
            CodingTermination::Timeout => {
                limitations.push("coding timeout reached".to_string());
            }
            CodingTermination::Cancelled => {
                limitations.push("coding cancelled".to_string());
            }
            CodingTermination::Error => {}
            CodingTermination::VerificationFailed => {
                limitations.push(format!(
                    "verification failed after {} revision(s); session changes rolled back",
                    self.revisions
                ));
            }
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
    fn test_list_output_paths() {
        let paths = list_output_paths("a.rs\nb.rs\n");
        assert_eq!(paths, vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")]);
    }

    #[test]
    fn test_render_testing_preserves_cancelled_machine_fact() {
        // A cancelled testing command must keep its machine fact visible to
        // the coder: exit code -1, success false AND the cancelled marker.
        let mut state = CodingState::new(
            CodingRequest::new("implement the fix", "."),
            CodingLimits::default(),
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
