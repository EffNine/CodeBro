//! The Testing request/result contract (Sprint 30D).
//!
//! These are the structured types exchanged between the coordinator and the
//! autonomous Testing subagent. The result deliberately preserves exact
//! machine facts — per-command exit codes, success, duration, timeouts and
//! denials — so the parent runtime consumes authoritative validation evidence,
//! never model prose.
//!
//! The core principle: the execution result belongs to the machine. A
//! `TestCommandResult.success` is derived exclusively from the process exit
//! code, never from a `contains("passed")` heuristic.

use std::fmt;
use std::path::PathBuf;

use crate::agent::grounding::GroundedContext;

use super::limits::TestingLimits;

/// How the testing session terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestingTermination {
    /// The subagent produced a final answer.
    Completed,
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
    /// A provider or tool error occurred.
    Error,
}

impl fmt::Display for TestingTermination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestingTermination::Completed => write!(f, "completed"),
            TestingTermination::IterationLimit => write!(f, "iteration_limit"),
            TestingTermination::ToolLimit => write!(f, "tool_limit"),
            TestingTermination::ModelLimit => write!(f, "model_limit"),
            TestingTermination::Timeout => write!(f, "timeout"),
            TestingTermination::Cancelled => write!(f, "cancelled"),
            TestingTermination::Error => write!(f, "error"),
        }
    }
}

impl TestingTermination {
    /// Whether the session produced a usable result.
    pub fn is_completed(&self) -> bool {
        matches!(self, TestingTermination::Completed)
    }
}

/// A request to the autonomous Testing subagent.
#[derive(Debug, Clone)]
pub struct TestingRequest {
    /// The testing objective.
    pub task: String,
    /// Absolute workspace root the subagent validates.
    pub workspace_root: PathBuf,
    /// Sprint 30B grounded context used as the subagent's initial knowledge.
    pub grounding: GroundedContext,
    /// Explicit session bounds.
    pub limits: TestingLimits,
}

impl TestingRequest {
    pub fn new(task: impl Into<String>, workspace_root: impl Into<PathBuf>) -> Self {
        TestingRequest {
            task: task.into(),
            workspace_root: workspace_root.into(),
            grounding: GroundedContext::default(),
            limits: TestingLimits::default(),
        }
    }

    pub fn with_grounding(mut self, grounding: GroundedContext) -> Self {
        self.grounding = grounding;
        self
    }

    pub fn with_limits(mut self, limits: TestingLimits) -> Self {
        self.limits = limits;
        self
    }
}

/// The authoritative record of one executed validation command.
///
/// `success` is derived ONLY from `exit_code == 0` (and the absence of
/// timeout/cancellation/denial). It is never derived from the output text:
/// a failing `cargo test` prints "0 passed; 1 failed", but its exit code 101
/// makes this record a failure.
#[derive(Debug, Clone)]
pub struct TestCommandResult {
    /// The command that was executed (already policy-checked).
    pub command: String,
    /// The working directory the command ran in.
    pub working_directory: String,
    /// The authoritative process exit code. `-1` means no process ran
    /// (denied, or the command never started).
    pub exit_code: i32,
    /// `exit_code == 0 && !timeout && !cancelled && !denied`.
    pub success: bool,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u128,
    /// Captured command output (PTY-merged stdout+stderr), redacted and
    /// truncated to the per-command cap.
    pub output: String,
    /// Whether the command exceeded the per-command PTY timeout.
    pub timeout: bool,
    /// Whether the command was terminated by cancellation.
    pub cancelled: bool,
    /// Whether the command was rejected by the Testing policy (never ran).
    pub denied: bool,
    /// The policy denial reason, when `denied`.
    pub denied_reason: Option<String>,
}

impl TestCommandResult {
    /// A record for a command that was denied before any process ran. The
    /// exit code is `-1` and `success` is `false` — a denial is a machine
    /// fact, not a model interpretation.
    pub fn denied(command: &str, working_directory: &str, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        TestCommandResult {
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
        }
    }

    /// A record for a command that never produced an exit code (spawn failure,
    /// error chunk). Treating it as a failure is the honest machine fact.
    pub fn failed_to_run(command: &str, working_directory: &str, error: &str) -> Self {
        TestCommandResult {
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
        }
    }

    /// A record for a command that hit the per-command PTY timeout.
    pub fn timed_out(command: &str, working_directory: &str, duration_ms: u128) -> Self {
        TestCommandResult {
            command: command.to_string(),
            working_directory: working_directory.to_string(),
            exit_code: -1,
            success: false,
            duration_ms,
            output: "[TIMED OUT] the command exceeded the per-command timeout".to_string(),
            timeout: true,
            cancelled: false,
            denied: false,
            denied_reason: None,
        }
    }

    /// The authoritative success predicate: the process exit code is the
    /// single source of truth. Prose that says "passed" is irrelevant.
    pub fn success_from_exit_code(exit_code: i32, timeout: bool, cancelled: bool) -> bool {
        exit_code == 0 && !timeout && !cancelled
    }

    /// A compact machine-readable rendering used in prompts and reports.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("$ {}\n", self.command));
        out.push_str(&format!("  exit_code: {}\n", self.exit_code));
        out.push_str(&format!("  success: {}\n", self.success));
        out.push_str(&format!("  duration_ms: {}\n", self.duration_ms));
        if self.denied {
            out.push_str(&format!(
                "  denied: {}",
                self.denied_reason.as_deref().unwrap_or("")
            ));
        } else if self.timeout {
            out.push_str("  timed_out: true");
        } else if self.cancelled {
            out.push_str("  cancelled: true");
        }
        if !self.output.trim().is_empty() {
            out.push_str(&format!("\n  output:\n{}\n", self.output));
        }
        out
    }
}

/// One real tool observation performed during testing (read-only tools and
/// the authoritative command records both become observations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestObservation {
    pub name: String,
    pub arguments: String,
    pub result: String,
    pub success: bool,
}

/// The kind of failure, using the existing build/test/lint/format taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestFailureKind {
    Build,
    Test,
    Lint,
    Format,
    Timeout,
    Command,
    Environment,
    Cancelled,
    Denied,
}

impl TestFailureKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestFailureKind::Build => "build_failure",
            TestFailureKind::Test => "test_failure",
            TestFailureKind::Lint => "lint_failure",
            TestFailureKind::Format => "format_failure",
            TestFailureKind::Timeout => "timeout",
            TestFailureKind::Command => "command_failure",
            TestFailureKind::Environment => "environment_failure",
            TestFailureKind::Cancelled => "cancelled",
            TestFailureKind::Denied => "denied",
        }
    }

    /// Classify a command record's failure kind from the command text and
    /// result state.
    pub fn from_command(command: &str, result: &TestCommandResult) -> TestFailureKind {
        if result.timeout {
            return TestFailureKind::Timeout;
        }
        if result.denied {
            return TestFailureKind::Denied;
        }
        let lower = command.to_lowercase();
        if lower.starts_with("cargo test") || lower.starts_with("cargo t ") {
            TestFailureKind::Test
        } else if lower.starts_with("cargo build") || lower.starts_with("cargo b ") {
            TestFailureKind::Build
        } else if lower.starts_with("cargo clippy") {
            TestFailureKind::Lint
        } else if lower.starts_with("cargo fmt") {
            TestFailureKind::Format
        } else {
            TestFailureKind::Command
        }
    }
}

/// One concrete validation failure with its machine facts.
#[derive(Debug, Clone)]
pub struct TestFailure {
    pub kind: TestFailureKind,
    pub command: String,
    pub exit_code: i32,
    pub output: String,
}

impl TestFailure {
    pub fn render(&self) -> String {
        format!(
            "- {} ({}) exit_code: {}\n{}",
            self.command,
            self.kind.as_str(),
            self.exit_code,
            self.output
        )
    }
}

/// One evidence-backed finding returned by testing.
#[derive(Debug, Clone)]
pub struct TestFinding {
    pub statement: String,
    pub kind: Option<TestFailureKind>,
    pub evidence: String,
}

/// A snapshot of the workspace git state, used to prove Testing never mutates
/// source or repository state.
#[derive(Debug, Clone, Default)]
pub struct GitStateSnapshot {
    pub has_git: bool,
    /// `git status --short` output.
    pub status: String,
    /// Whether the tracked tree is clean (`git diff --check` empty and
    /// `status` contains no tracked modifications).
    pub clean: bool,
}

/// The structured result of one testing session.
#[derive(Debug, Clone)]
pub struct TestingResult {
    /// Human-readable executive summary (model prose — advisory only).
    pub summary: String,
    /// Evidence-backed findings.
    pub findings: Vec<TestFinding>,
    /// Authoritative per-command records, in execution order. These are the
    /// machine facts the parent consumes.
    pub commands_run: Vec<TestCommandResult>,
    /// Files actually inspected via tools.
    pub files_inspected: Vec<PathBuf>,
    /// Concrete validation failures derived from failed command records.
    pub failures: Vec<TestFailure>,
    /// Total real tool calls executed (model-requested).
    pub tool_calls: usize,
    /// Number of reasoning iterations.
    pub iterations: usize,
    /// Number of model (provider) calls.
    pub model_calls: usize,
    /// Why the session terminated.
    pub termination: TestingTermination,
    /// Whether the final prose synthesis was actually produced.
    pub synthesis_complete: bool,
    /// Ordered tool observations (evidence trail).
    pub observations: Vec<TestObservation>,
    /// Explicit limitations of this testing pass.
    pub limitations: Vec<String>,
    /// Wall-clock duration.
    pub duration_ms: u64,
    /// Approximate output size in bytes.
    pub output_size: usize,
    /// Provider that executed the testing.
    pub provider: String,
    /// Model used.
    pub model: String,
    /// Git state before testing ran (defense-in-depth no-mutation proof).
    pub git_before: Option<GitStateSnapshot>,
    /// Git state after testing ran.
    pub git_after: Option<GitStateSnapshot>,
}

impl TestingResult {
    /// A result for a session that never executed (e.g. provider failure).
    pub fn failed(task: &str, termination: TestingTermination, error: &str) -> Self {
        TestingResult {
            summary: format!("Testing for '{task}' did not complete: {error}"),
            findings: Vec::new(),
            commands_run: Vec::new(),
            files_inspected: Vec::new(),
            failures: Vec::new(),
            tool_calls: 0,
            iterations: 0,
            model_calls: 0,
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

    /// Whether the git tree was mutated between the before and after
    /// snapshots. Ignored build artifacts (e.g. `target/`) do not count: only
    /// tracked-file / index / repository-metadata changes matter.
    pub fn git_tree_unchanged(&self) -> bool {
        match (&self.git_before, &self.git_after) {
            (Some(before), Some(after)) => {
                before.status == after.status && before.clean == after.clean
            }
            _ => true,
        }
    }

    /// Human-readable rendering used for the `testing` ContextFragment that
    /// reaches the main LLM prompt. The machine-readable command facts are
    /// authoritative and always present.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "## Autonomous Testing Findings\n\nTermination: {}\n",
            self.termination
        ));
        out.push_str(&format!(
            "Iterations: {} | tool calls: {} | model calls: {} | commands run: {} | synthesis complete: {}\n",
            self.iterations,
            self.tool_calls,
            self.model_calls,
            self.commands_run.len(),
            self.synthesis_complete
        ));
        if !self.provider.is_empty() {
            out.push_str(&format!("Provider: {}\n", self.provider));
        }
        if !self.files_inspected.is_empty() {
            out.push_str(&format!(
                "Files inspected: {}\n",
                self.files_inspected
                    .iter()
                    .map(|f| f.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(git) = &self.git_after {
            out.push_str(&format!("Git tree clean after: {}\n", git.clean));
        }

        out.push_str("\nCommand results (authoritative exit codes):\n");
        if self.commands_run.is_empty() {
            out.push_str("(no commands executed)\n");
        } else {
            for command in &self.commands_run {
                out.push_str(&command.render());
            }
        }

        if !self.failures.is_empty() {
            out.push_str("\nFailures:\n");
            for failure in &self.failures {
                out.push_str(&failure.render());
            }
        }

        if !self.findings.is_empty() {
            out.push_str("\nFindings:\n");
            for finding in &self.findings {
                out.push_str(&format!("- {}\n", finding.statement));
            }
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
            "[testing] provider={} model={} iterations={} tool_calls={} model_calls={} commands={} failures={} files={} termination={} synthesis={} duration={}ms output={}B",
            if self.provider.is_empty() { "-" } else { &self.provider },
            if self.model.is_empty() { "-" } else { &self.model },
            self.iterations,
            self.tool_calls,
            self.model_calls,
            self.commands_run.len(),
            self.failures.len(),
            self.files_inspected.len(),
            self.termination,
            self.synthesis_complete,
            self.duration_ms,
            self.output_size,
        )
    }

    /// Record the provider/model that executed the testing.
    pub fn with_provider(mut self, provider: String, model: String) -> Self {
        self.provider = provider;
        self.model = model;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_termination_display_values() {
        assert_eq!(TestingTermination::Completed.to_string(), "completed");
        assert_eq!(
            TestingTermination::IterationLimit.to_string(),
            "iteration_limit"
        );
        assert_eq!(TestingTermination::ToolLimit.to_string(), "tool_limit");
        assert_eq!(TestingTermination::ModelLimit.to_string(), "model_limit");
        assert_eq!(TestingTermination::Timeout.to_string(), "timeout");
        assert_eq!(TestingTermination::Cancelled.to_string(), "cancelled");
        assert_eq!(TestingTermination::Error.to_string(), "error");
    }

    #[test]
    fn test_exit_code_is_authoritative_over_prose() {
        // Output contains "passed" but the exit code is non-zero → FAILURE.
        let result = TestCommandResult {
            command: "cargo test".to_string(),
            working_directory: "/repo".to_string(),
            exit_code: 101,
            success: TestCommandResult::success_from_exit_code(101, false, false),
            duration_ms: 500,
            output: "0 passed; 1 failed".to_string(),
            timeout: false,
            cancelled: false,
            denied: false,
            denied_reason: None,
        };
        assert!(
            !result.success,
            "exit code 101 must produce success=false even though output says 'passed'"
        );

        // Output contains "failed" but the exit code is zero → SUCCESS.
        let result = TestCommandResult {
            command: "cargo test".to_string(),
            working_directory: "/repo".to_string(),
            exit_code: 0,
            success: TestCommandResult::success_from_exit_code(0, false, false),
            duration_ms: 500,
            output: "test always_failed ... ok\n1 passed".to_string(),
            timeout: false,
            cancelled: false,
            denied: false,
            denied_reason: None,
        };
        assert!(
            result.success,
            "exit code 0 must produce success=true even though output says 'failed'"
        );
    }

    #[test]
    fn test_denied_record_is_a_failure() {
        let record = TestCommandResult::denied("rm -rf /", "/repo", "denied by policy");
        assert!(record.denied);
        assert!(!record.success);
        assert_eq!(record.exit_code, -1);
    }

    #[test]
    fn test_failure_kind_classification() {
        let passing = TestCommandResult {
            command: "cargo test".to_string(),
            working_directory: "/r".to_string(),
            exit_code: 101,
            success: false,
            duration_ms: 1,
            output: String::new(),
            timeout: false,
            cancelled: false,
            denied: false,
            denied_reason: None,
        };
        assert_eq!(
            TestFailureKind::from_command("cargo test", &passing),
            TestFailureKind::Test
        );
        assert_eq!(
            TestFailureKind::from_command("cargo build", &passing),
            TestFailureKind::Build
        );
        assert_eq!(
            TestFailureKind::from_command("cargo clippy", &passing),
            TestFailureKind::Lint
        );
        assert_eq!(
            TestFailureKind::from_command("cargo fmt -- --check", &passing),
            TestFailureKind::Format
        );
        let timeout = TestCommandResult::timed_out("cargo test", "/r", 1000);
        assert_eq!(
            TestFailureKind::from_command("cargo test", &timeout),
            TestFailureKind::Timeout
        );
        let denied = TestCommandResult::denied("git commit", "/r", "denied".to_string());
        assert_eq!(
            TestFailureKind::from_command("git commit", &denied),
            TestFailureKind::Denied
        );
    }

    #[test]
    fn test_failed_result_is_bounded_error_result() {
        let result = TestingResult::failed(
            "validate the crate",
            TestingTermination::Error,
            "provider unavailable",
        );
        assert_eq!(result.termination, TestingTermination::Error);
        assert!(result.summary.contains("provider unavailable"));
        assert!(result.commands_run.is_empty());
        assert_eq!(result.tool_calls, 0);
        assert!(!result.synthesis_complete);
    }

    #[test]
    fn test_render_includes_authoritative_command_facts() {
        let result = TestingResult {
            summary: "tests pass".to_string(),
            findings: Vec::new(),
            commands_run: vec![TestCommandResult {
                command: "cargo test".to_string(),
                working_directory: "/repo".to_string(),
                exit_code: 101,
                success: false,
                duration_ms: 1200,
                output: "0 passed; 1 failed".to_string(),
                timeout: false,
                cancelled: false,
                denied: false,
                denied_reason: None,
            }],
            files_inspected: Vec::new(),
            failures: vec![TestFailure {
                kind: TestFailureKind::Test,
                command: "cargo test".to_string(),
                exit_code: 101,
                output: "0 passed; 1 failed".to_string(),
            }],
            tool_calls: 1,
            iterations: 2,
            model_calls: 2,
            termination: TestingTermination::Completed,
            synthesis_complete: true,
            observations: Vec::new(),
            limitations: Vec::new(),
            duration_ms: 5000,
            output_size: 200,
            provider: "mock".to_string(),
            model: "mock-model".to_string(),
            git_before: None,
            git_after: None,
        };
        let rendered = result.render();
        assert!(rendered.contains("Autonomous Testing Findings"));
        assert!(rendered.contains("exit_code: 101"));
        assert!(rendered.contains("success: false"));
        assert!(rendered.contains("0 passed; 1 failed"));
        assert!(rendered.contains("cargo test"));
    }

    #[test]
    fn test_git_tree_unchanged() {
        let mut result = TestingResult::failed("x", TestingTermination::Completed, "no-op");
        // No git → considered unchanged (vacuously).
        assert!(result.git_tree_unchanged());
        result.git_before = Some(GitStateSnapshot {
            has_git: true,
            status: String::new(),
            clean: true,
        });
        result.git_after = Some(GitStateSnapshot {
            has_git: true,
            status: String::new(),
            clean: true,
        });
        assert!(result.git_tree_unchanged());
        result.git_after = Some(GitStateSnapshot {
            has_git: true,
            status: " M src/main.rs".to_string(),
            clean: false,
        });
        assert!(!result.git_tree_unchanged());
    }
}
