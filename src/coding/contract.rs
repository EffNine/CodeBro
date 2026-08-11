//! The Coding request/result contract (Sprint 30F).
//!
//! These are the structured types exchanged between the coordinator and the
//! autonomous Coding subagent. Coding consumes the evidence of Research
//! (files, symbols, findings), Testing (commands, exit codes, failures) and —
//! crucially — the REAL `PlanningResult` (not rendered prose) so plan
//! adherence can be enforced mechanically, and produces an auditable record of
//! every repository mutation it applied, every verification it ran and how the
//! session terminated.
//!
//! The contract deliberately separates:
//! - machine facts (applied changes with previews, authoritative exit codes)
//!   from
//! - model reasoning (the final prose summary).
//!
//! Every [`AppliedChange`] carries its preview (readable diff), the original
//! content captured when the change was prepared (the rollback source), whether
//! it was planned, and whether it was verified. A verification record carries
//! the authoritative exit code — success is derived from the exit code, never
//! from command output text.

use std::fmt;
use std::path::PathBuf;

use crate::agent::grounding::GroundedContext;
use crate::planning::{PlanStep, PlanningResult};
use crate::research::ResearchResult;
use crate::testing::{TestObservation, TestingResult};

use super::limits::CodingLimits;

/// How the coding session terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingTermination {
    /// The subagent applied its changes and every applied change was covered
    /// by an authoritative successful verification.
    Completed,
    /// The subagent applied changes but the plan carried no validation
    /// commands, so NO machine verification ran. This is NOT
    /// completed-as-verified: the applied changes remain in the tree, honestly
    /// marked unverified (`verified == false`, `all_verified() == false`).
    VerificationUnavailable,
    /// The iteration budget was exhausted.
    IterationLimit,
    /// The tool-call budget was exhausted.
    ToolLimit,
    /// The model-call budget was exhausted.
    ModelLimit,
    /// The wall-clock timeout fired.
    Timeout,
    /// Cancellation was requested.
    Cancelled,
    /// A provider or tool error occurred (changes rolled back).
    Error,
    /// Verification failed beyond the revision budget (changes rolled back).
    VerificationFailed,
}

impl fmt::Display for CodingTermination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodingTermination::Completed => write!(f, "completed"),
            CodingTermination::VerificationUnavailable => write!(f, "verification_unavailable"),
            CodingTermination::IterationLimit => write!(f, "iteration_limit"),
            CodingTermination::ToolLimit => write!(f, "tool_limit"),
            CodingTermination::ModelLimit => write!(f, "model_limit"),
            CodingTermination::Timeout => write!(f, "timeout"),
            CodingTermination::Cancelled => write!(f, "cancelled"),
            CodingTermination::Error => write!(f, "error"),
            CodingTermination::VerificationFailed => write!(f, "verification_failed"),
        }
    }
}

impl CodingTermination {
    /// Whether the session produced a usable, machine-verified result.
    ///
    /// ONLY [`CodingTermination::Completed`] counts. A session that applied
    /// changes but could not run any authoritative verification
    /// ([`CodingTermination::VerificationUnavailable`]) is never
    /// "completed-as-verified", no matter what the model's report claims.
    pub fn is_completed(&self) -> bool {
        matches!(self, CodingTermination::Completed)
    }

    /// Whether this termination requires the session's own changes to be
    /// rolled back: a hard failure (verification exhausted or an error).
    ///
    /// [`CodingTermination::VerificationUnavailable`] intentionally does NOT
    /// roll back: the session's applied changes are real work that must stay
    /// inspectable, and they are reported honestly as unverified so no caller
    /// can mistake them for machine-verified.
    pub fn requires_rollback(&self) -> bool {
        matches!(
            self,
            CodingTermination::VerificationFailed | CodingTermination::Error
        )
    }
}

/// A request to the autonomous Coding subagent.
#[derive(Debug, Clone)]
pub struct CodingRequest {
    /// The coding objective.
    pub task: String,
    /// Absolute workspace root the subagent may mutate.
    pub workspace_root: PathBuf,
    /// Sprint 30B grounded context used as the subagent's initial knowledge.
    pub grounding: GroundedContext,
    /// Optional Sprint 30C research evidence.
    pub research: Option<ResearchResult>,
    /// Optional Sprint 30D testing evidence (authoritative machine facts).
    pub testing: Option<TestingResult>,
    /// The REAL Sprint 30E implementation plan (step files + validation). This
    /// is the plan-adherence authority, consumed as structured data.
    pub planning: Option<PlanningResult>,
    /// Explicit session bounds.
    pub limits: CodingLimits,
}

impl CodingRequest {
    pub fn new(task: impl Into<String>, workspace_root: impl Into<PathBuf>) -> Self {
        CodingRequest {
            task: task.into(),
            workspace_root: workspace_root.into(),
            grounding: GroundedContext::default(),
            research: None,
            testing: None,
            planning: None,
            limits: CodingLimits::default(),
        }
    }

    pub fn with_grounding(mut self, grounding: GroundedContext) -> Self {
        self.grounding = grounding;
        self
    }

    pub fn with_research(mut self, research: Option<ResearchResult>) -> Self {
        self.research = research;
        self
    }

    pub fn with_testing(mut self, testing: Option<TestingResult>) -> Self {
        self.testing = testing;
        self
    }

    pub fn with_planning(mut self, planning: Option<PlanningResult>) -> Self {
        self.planning = planning;
        self
    }

    pub fn with_limits(mut self, limits: CodingLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The union of every file the plan touches (the plan-adherence boundary).
    pub fn planned_files(&self) -> Vec<PathBuf> {
        self.planning
            .as_ref()
            .map(|p| p.affected_files.clone())
            .unwrap_or_default()
    }

    /// The ordered validation commands the plan requires, deduplicated.
    pub fn plan_validation_commands(&self) -> Vec<String> {
        let mut commands = Vec::new();
        if let Some(planning) = &self.planning {
            for step in &planning.plan {
                for command in &step.validation {
                    if !commands.contains(command) {
                        commands.push(command.clone());
                    }
                }
            }
        }
        commands
    }
}

/// The plan steps themselves, exposed for prompt rendering.
impl CodingRequest {
    pub fn plan_steps(&self) -> &[PlanStep] {
        self.planning
            .as_ref()
            .map(|p| p.plan.as_slice())
            .unwrap_or(&[])
    }
}

/// One repository mutation the session applied (and why/how).
///
/// `backup` is the complete original content captured when the change was
/// prepared — the rollback source. `created` means the file did not exist
/// before. `unplanned` means the target file was not named by the plan (the
/// deviation is recorded, never silent). `verified` is set only after an
/// authoritative successful verification covered this change.
#[derive(Debug, Clone)]
pub struct AppliedChange {
    /// The mutated file, relative to the workspace root.
    pub path: PathBuf,
    /// Whether the file was created (did not exist before).
    pub created: bool,
    /// Whether the target was outside the plan's affected files.
    pub unplanned: bool,
    /// Readable diff preview of the change.
    pub preview: String,
    /// Original full content before the change (rollback source; empty for
    /// created files).
    pub backup: String,
    /// The full content the session wrote (used to prove a created file is
    /// still the session's own work before removing it on rollback).
    pub full_new: String,
    /// Whether an authoritative successful verification covered this change.
    ///
    /// Set ONLY by a machine verification that actually ran and succeeded
    /// (exit code 0) — an explicit `verify` call or the completion gate. It is
    /// NEVER set because the model finished or produced a successful-sounding
    /// report. When the session terminates as
    /// [`CodingTermination::VerificationUnavailable`], no change is verified.
    pub verified: bool,
    /// Whether this change was rolled back by a terminal failure.
    pub rolled_back: bool,
}

impl AppliedChange {
    /// The optimistic status word for the render (does NOT account for a
    /// later rollback — [`AppliedChange::status`] does).
    pub fn status(&self) -> &'static str {
        if self.rolled_back {
            "rolled_back"
        } else if self.verified {
            "verified"
        } else if self.created {
            "created"
        } else {
            "applied"
        }
    }
}

/// Why a verification command ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationSource {
    /// The model explicitly called `verify`.
    Explicit,
    /// The completion gate auto-verified the session's unverified changes.
    CompletionGate,
}

impl fmt::Display for VerificationSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerificationSource::Explicit => write!(f, "explicit"),
            VerificationSource::CompletionGate => write!(f, "completion_gate"),
        }
    }
}

/// One authoritative verification record.
///
/// `success` is derived ONLY from `exit_code == 0` (and the absence of
/// timeout/cancellation/denial). It is never derived from output text.
#[derive(Debug, Clone)]
pub struct VerificationRecord {
    /// The command that was executed (already policy-checked).
    pub command: String,
    /// The working directory the command ran in.
    pub working_directory: String,
    /// The authoritative process exit code. `-1` means no process ran.
    pub exit_code: i32,
    /// `exit_code == 0 && !timeout && !cancelled && !denied`.
    pub success: bool,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u128,
    /// Captured command output, redacted and truncated.
    pub output: String,
    /// Whether the command exceeded the per-command PTY timeout.
    pub timeout: bool,
    /// Whether the command was terminated by cancellation.
    pub cancelled: bool,
    /// Whether the command was rejected by the Testing policy (never ran).
    pub denied: bool,
    /// The policy denial reason, when `denied`.
    pub denied_reason: Option<String>,
    /// Whether the model requested this or the completion gate ran it.
    pub source: VerificationSource,
}

impl VerificationRecord {
    /// A record for a command that was denied before any process ran.
    pub fn denied(
        command: &str,
        working_directory: &str,
        reason: String,
        source: VerificationSource,
    ) -> Self {
        VerificationRecord {
            command: command.to_string(),
            working_directory: working_directory.to_string(),
            exit_code: -1,
            success: false,
            duration_ms: 0,
            output: format!("[DENIED] {reason}"),
            timeout: false,
            cancelled: false,
            denied: true,
            denied_reason: Some(reason),
            source,
        }
    }

    /// A record for a command that never produced an exit code.
    pub fn failed_to_run(
        command: &str,
        working_directory: &str,
        error: &str,
        source: VerificationSource,
    ) -> Self {
        VerificationRecord {
            command: command.to_string(),
            working_directory: working_directory.to_string(),
            exit_code: -1,
            success: false,
            duration_ms: 0,
            output: format!("[ERROR] {error}"),
            timeout: false,
            cancelled: false,
            denied: false,
            denied_reason: None,
            source,
        }
    }

    /// A compact machine-readable rendering used in prompts and reports.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("  $ {} (source: {})\n", self.command, self.source));
        out.push_str(&format!("    exit_code: {}\n", self.exit_code));
        out.push_str(&format!("    success: {}\n", self.success));
        if self.denied {
            out.push_str(&format!(
                "    denied: {}",
                self.denied_reason.as_deref().unwrap_or("")
            ));
        } else if self.timeout {
            out.push_str("    timed_out: true");
        } else if self.cancelled {
            out.push_str("    cancelled: true");
        }
        if !self.output.trim().is_empty() {
            out.push_str(&format!("\n    output:\n{}\n", indent_output(&self.output)));
        }
        out
    }
}

/// One real tool observation performed during coding (read-only tools and
/// the mutation/verification events both become observations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingObservation {
    pub name: String,
    pub arguments: String,
    pub result: String,
    pub success: bool,
}

/// The structured result of one coding session.
#[derive(Debug, Clone)]
pub struct CodingResult {
    /// Human-readable executive summary (model prose — advisory only).
    pub summary: String,
    /// Every change the session applied, in application order.
    pub changes: Vec<AppliedChange>,
    /// Changes that were applied outside the plan's affected files
    /// (recorded, never silent).
    pub unplanned_changes: Vec<AppliedChange>,
    /// Authoritative verification records, in execution order.
    pub verification: Vec<VerificationRecord>,
    /// Files actually inspected via tools.
    pub files_inspected: Vec<PathBuf>,
    /// Total real tool calls executed (model-requested).
    pub tool_calls: usize,
    /// Number of reasoning iterations.
    pub iterations: usize,
    /// Number of model (provider) calls.
    pub model_calls: usize,
    /// Number of failed verification attempts before termination.
    pub revisions: usize,
    /// Why the session terminated.
    pub termination: CodingTermination,
    /// Whether the final prose synthesis was actually produced.
    pub synthesis_complete: bool,
    /// Ordered tool observations (evidence trail).
    pub observations: Vec<CodingObservation>,
    /// Explicit limitations of this coding pass (incl. rollback log).
    pub limitations: Vec<String>,
    /// Wall-clock duration.
    pub duration_ms: u64,
    /// Approximate output size in bytes.
    pub output_size: usize,
    /// Provider that executed the coding.
    pub provider: String,
    /// Model used.
    pub model: String,
    /// Git tracked state before coding ran (baseline for observability).
    pub git_before: Option<crate::testing::GitStateSnapshot>,
    /// Git tracked state after coding ran.
    pub git_after: Option<crate::testing::GitStateSnapshot>,
}

impl CodingResult {
    /// A result for a session that never executed (e.g. provider failure).
    pub fn failed(task: &str, termination: CodingTermination, error: &str) -> Self {
        CodingResult {
            summary: format!("Coding for '{task}' did not complete: {error}"),
            changes: Vec::new(),
            unplanned_changes: Vec::new(),
            verification: Vec::new(),
            files_inspected: Vec::new(),
            tool_calls: 0,
            iterations: 0,
            model_calls: 0,
            revisions: 0,
            termination,
            synthesis_complete: false,
            observations: Vec::new(),
            limitations: vec![error.to_string()],
            duration_ms: 0,
            output_size: 0,
            provider: String::new(),
            model: String::new(),
            git_before: None,
            git_after: None,
        }
    }

    /// Whether every applied change was actually covered by a successful
    /// authoritative verification (vacuously true when nothing was applied).
    ///
    /// This is a MACHINE FACT, not a proxy for "the model finished": it is
    /// true ONLY when each change's `verified` flag was set by an
    /// authoritative exit-code-0 verification (an explicit `verify` or the
    /// completion gate). A session that applied changes but could not run any
    /// validation command terminates as
    /// [`CodingTermination::VerificationUnavailable`] and NEVER satisfies
    /// `all_verified()`.
    pub fn all_verified(&self) -> bool {
        self.changes.iter().all(|c| c.verified)
    }

    /// Human-readable rendering used for the `coding` ContextFragment that
    /// reaches the main LLM prompt. Preserves the applied changes with their
    /// previews, the verification records and the deviations.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "## Autonomous Coding\n\nTermination: {}\n",
            self.termination
        ));
        out.push_str(&format!(
            "Iterations: {} | tool calls: {} | model calls: {} | revisions: {} | changes: {} | verification runs: {} | synthesis complete: {}\n",
            self.iterations,
            self.tool_calls,
            self.model_calls,
            self.revisions,
            self.changes.len(),
            self.verification.len(),
            self.synthesis_complete
        ));
        if !self.provider.is_empty() {
            out.push_str(&format!("Provider: {}\n", self.provider));
        }

        out.push_str("\n## Applied changes\n");
        if self.changes.is_empty() {
            out.push_str("(no changes applied)\n");
        } else {
            for change in &self.changes {
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

        if !self.unplanned_changes.is_empty() {
            out.push_str("\n## Unplanned changes (deviation from the plan, recorded)\n");
            for change in &self.unplanned_changes {
                out.push_str(&format!(
                    "- {} [{}]\n",
                    change.path.display(),
                    change.status()
                ));
            }
        }

        out.push_str("\n## Verification\n");
        if self.verification.is_empty() {
            out.push_str("(no verification commands executed)\n");
        } else {
            for record in &self.verification {
                out.push_str(&record.render());
            }
        }

        if !self.files_inspected.is_empty() {
            out.push_str(&format!(
                "\nFiles inspected: {}\n",
                self.files_inspected
                    .iter()
                    .map(|f| f.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        if !self.summary.is_empty() {
            out.push_str(&format!("\nSummary:\n{}\n", self.summary));
        }
        if !self.limitations.is_empty() {
            out.push_str("\nLimitations:\n");
            for limitation in &self.limitations {
                out.push_str(&format!("- {}\n", limitation));
            }
        }
        out
    }

    /// Compact one-line summary for observability.
    pub fn summary_line(&self) -> String {
        format!(
            "[coding] provider={} model={} iterations={} tool_calls={} model_calls={} revisions={} changes={} unplanned={} verification={} termination={} synthesis={} duration={}ms output={}B",
            if self.provider.is_empty() { "-" } else { &self.provider },
            if self.model.is_empty() { "-" } else { &self.model },
            self.iterations,
            self.tool_calls,
            self.model_calls,
            self.revisions,
            self.changes.len(),
            self.unplanned_changes.len(),
            self.verification.len(),
            self.termination,
            self.synthesis_complete,
            self.duration_ms,
            self.output_size,
        )
    }

    /// Record the provider/model that executed the coding.
    pub fn with_provider(mut self, provider: String, model: String) -> Self {
        self.provider = provider;
        self.model = model;
        self
    }
}

fn indent_output(text: &str) -> String {
    text.lines()
        .map(|l| format!("      {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_termination_display_values() {
        assert_eq!(CodingTermination::Completed.to_string(), "completed");
        assert_eq!(
            CodingTermination::VerificationUnavailable.to_string(),
            "verification_unavailable"
        );
        assert_eq!(
            CodingTermination::IterationLimit.to_string(),
            "iteration_limit"
        );
        assert_eq!(CodingTermination::ToolLimit.to_string(), "tool_limit");
        assert_eq!(CodingTermination::ModelLimit.to_string(), "model_limit");
        assert_eq!(CodingTermination::Timeout.to_string(), "timeout");
        assert_eq!(CodingTermination::Cancelled.to_string(), "cancelled");
        assert_eq!(CodingTermination::Error.to_string(), "error");
        assert_eq!(
            CodingTermination::VerificationFailed.to_string(),
            "verification_failed"
        );
    }

    #[test]
    fn test_termination_rollback_policy() {
        assert!(CodingTermination::VerificationFailed.requires_rollback());
        assert!(CodingTermination::Error.requires_rollback());
        for termination in [
            CodingTermination::Completed,
            CodingTermination::VerificationUnavailable,
            CodingTermination::IterationLimit,
            CodingTermination::ToolLimit,
            CodingTermination::ModelLimit,
            CodingTermination::Timeout,
            CodingTermination::Cancelled,
        ] {
            assert!(
                !termination.requires_rollback(),
                "{termination} must not require rollback"
            );
        }
    }

    #[test]
    fn test_request_plan_surface_is_structured() {
        let planning = PlanningResult::failed(
            "x",
            crate::planning::PlanningTermination::Completed,
            "no-op",
        );
        let mut request = CodingRequest::new("implement", "/repo")
            .with_planning(Some(planning))
            .with_limits(CodingLimits::default());
        assert_eq!(request.planned_files(), Vec::<PathBuf>::new());
        assert!(request.plan_validation_commands().is_empty());

        request.planning = Some(make_plan());
        assert_eq!(request.planned_files(), vec![PathBuf::from("src/lib.rs")]);
        assert_eq!(
            request.plan_validation_commands(),
            vec!["cargo check".to_string(), "cargo test".to_string()]
        );
        assert_eq!(request.plan_steps().len(), 1);
    }

    #[test]
    fn test_failed_result_is_bounded_error_result() {
        let result = CodingResult::failed(
            "implement the change",
            CodingTermination::Error,
            "provider unavailable",
        );
        assert_eq!(result.termination, CodingTermination::Error);
        assert!(result.summary.contains("provider unavailable"));
        assert!(result.changes.is_empty());
        assert!(result.verification.is_empty());
        assert_eq!(result.tool_calls, 0);
        assert!(!result.synthesis_complete);
    }

    #[test]
    fn test_verification_unavailable_is_never_completed() {
        assert!(!CodingTermination::VerificationUnavailable.is_completed());
        assert!(!CodingTermination::VerificationUnavailable.requires_rollback());
        // Applied-but-unverified changes never satisfy all_verified().
        let mut result =
            CodingResult::failed("x", CodingTermination::VerificationUnavailable, "no-op");
        result.changes.push(AppliedChange {
            path: PathBuf::from("src/lib.rs"),
            created: false,
            unplanned: false,
            preview: "diff".to_string(),
            backup: "old".to_string(),
            full_new: "new".to_string(),
            verified: false,
            rolled_back: false,
        });
        assert!(!result.all_verified());
        assert!(result.changes[0].status() == "applied");
    }

    #[test]
    fn test_all_verified_reflects_change_verification() {
        let mut result = CodingResult::failed("x", CodingTermination::Completed, "no-op");
        assert!(result.all_verified(), "no changes → vacuously verified");
        result.changes.push(AppliedChange {
            path: PathBuf::from("src/lib.rs"),
            created: false,
            unplanned: false,
            preview: "diff".to_string(),
            backup: "old".to_string(),
            full_new: "new".to_string(),
            verified: true,
            rolled_back: false,
        });
        assert!(result.all_verified());
        result.changes[0].verified = false;
        assert!(!result.all_verified());
    }

    #[test]
    fn test_render_includes_changes_verification_and_deviations() {
        let mut result = CodingResult::failed("x", CodingTermination::Completed, "no-op");
        result.changes.push(AppliedChange {
            path: PathBuf::from("src/extra.rs"),
            created: true,
            unplanned: true,
            preview: "+fn extra() {}".to_string(),
            backup: String::new(),
            full_new: "fn extra() {}".to_string(),
            verified: false,
            rolled_back: false,
        });
        result.unplanned_changes.push(AppliedChange {
            path: PathBuf::from("src/extra.rs"),
            created: true,
            unplanned: true,
            preview: "+fn extra() {}".to_string(),
            backup: String::new(),
            full_new: "fn extra() {}".to_string(),
            verified: false,
            rolled_back: false,
        });
        result.verification.push(VerificationRecord {
            command: "cargo check".to_string(),
            working_directory: "/repo".to_string(),
            exit_code: 0,
            success: true,
            duration_ms: 10,
            output: String::new(),
            timeout: false,
            cancelled: false,
            denied: false,
            denied_reason: None,
            source: VerificationSource::Explicit,
        });
        result.summary = "implemented subtract".to_string();

        let rendered = result.render();
        assert!(rendered.contains("Autonomous Coding"));
        assert!(rendered.contains("src/extra.rs"));
        assert!(rendered.contains("[created]"));
        assert!(rendered.contains("[UNPLANNED]"));
        assert!(rendered.contains("## Unplanned changes"));
        assert!(rendered.contains("cargo check"));
        assert!(rendered.contains("exit_code: 0"));
        assert!(rendered.contains("implemented subtract"));
    }

    fn make_plan() -> PlanningResult {
        PlanningResult {
            summary: "plan".to_string(),
            plan: vec![crate::planning::PlanStep {
                order: 1,
                action: "modify add".to_string(),
                target_files: vec![PathBuf::from("src/lib.rs")],
                target_symbols: vec![],
                rationale: "test".to_string(),
                dependencies: vec![],
                validation: vec!["cargo check".to_string(), "cargo test".to_string()],
                risk: "test".to_string(),
                evidence: vec![],
            }],
            affected_files: vec![PathBuf::from("src/lib.rs")],
            affected_symbols: vec![],
            dependencies: vec![],
            tests_to_update: vec![],
            risks: vec![],
            assumptions: vec![],
            evidence: vec![],
            tool_calls: 0,
            iterations: 0,
            model_calls: 0,
            termination: crate::planning::PlanningTermination::Completed,
            synthesis_complete: true,
            tool_observations: vec![],
            limitations: vec![],
            duration_ms: 0,
            output_size: 0,
            provider: String::new(),
            model: String::new(),
        }
    }
}
