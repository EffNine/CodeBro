//! Bounded coding limits (Sprint 30F).
//!
//! The autonomous Coding subagent gets its own explicit budget, strictly
//! independent of the main agent's and of the Research/Testing/Planning
//! budgets. Coding is the ONLY mutating subagent, so its budget deliberately
//! matches Testing's conservative profile: real verification commands can take
//! seconds, but a session must stay small, bounded and reversible.

/// Maximum number of coding reasoning iterations before giving up.
///
/// Mirrors the Research/Testing budgets. 6 lets a real LLM verify a plan step
/// with a targeted read, apply changes and run the necessary verification
/// commands while still leaving room for the reserved final synthesis call.
pub const MAX_CODING_ITERATIONS: usize = 6;

/// Maximum total tool calls across the entire coding session.
///
/// Coding applies and verifies — it does not explore. 12 keeps a session tight
/// while still permitting the `read → propose_change → verify` flow with one
/// bounded revision.
pub const MAX_CODING_TOOL_CALLS: usize = 12;

/// Maximum total model (provider) calls across the coding session.
pub const MAX_CODING_MODEL_CALLS: usize = 6;

/// Model calls reserved for the final synthesis step.
///
/// The coding loop divides its model budget into an execution phase (up to
/// `max_model_calls - reserved_synthesis_calls`) and a guaranteed final
/// synthesis call. A model that keeps mutating can never starve the final
/// report.
pub const RESERVED_SYNTHESIS_CALLS: usize = 1;

/// Hard wall-clock timeout for a coding session, in milliseconds.
///
/// Matches the main task default and the Research/Testing sessions. The
/// session deadline is authoritative: a hanging verification command cannot
/// hang the main CodeBro task (the per-command PTY timeout catches it earlier).
pub const CODING_TIMEOUT_MS: u64 = 30_000;

/// Per-command PTY timeout, in seconds (identical to Testing).
///
/// Verification commands run under the existing PTY timeout mechanism, so a
/// single hung command terminates before the session deadline.
pub const COMMAND_TIMEOUT_SECS: u64 = 60;

/// Maximum accumulated output retained across the whole coding session (in
/// bytes). Coding must stay cheap; oversized diffs and command output are
/// truncated.
pub const MAX_CODING_OUTPUT_BYTES: usize = 16 * 1024;

/// Maximum characters retained from a single tool result or change preview.
///
/// The cap never affects the exit code — the machine fact stays authoritative.
pub const MAX_TOOL_RESULT_CHARS: usize = 8 * 1024;

/// Maximum failed-verification revision attempts before the session fails and
/// its own changes are rolled back.
///
/// Coding does not re-enter its loop from scratch after a failure; the
/// revision budget bounds how many failed `verify` outcomes the model may
/// recover from before the session terminates as `VerificationFailed` and
/// rolls back. Matching the main loop's default revision budget.
pub const MAX_REVISION_ATTEMPTS: usize = 2;

/// The tool names the Coding subagent is explicitly allowed to call.
///
/// Coding is the FIRST mutating subagent, and its execution surface stays the
/// narrowest of all: read-only inspection plus `propose_change` (mutation
/// through [`ChangePlan`](crate::tools::ChangePlan) behind the change-engine
/// boundary) and `verify` (runtime-intercepted, policy-checked through
/// [`TestingTooling`](crate::testing::TestingTooling) — Coding has NO
/// `run_command`). Raw filesystem tools (`create_file`, `edit_file`) and git
/// mutations are never allowed.
pub const CODING_ALLOWED_TOOLS: &[&str] = &[
    "list_files",
    "read_file",
    "git_status",
    "git_diff",
    "propose_change",
    "verify",
];

/// Coding limits applied to one coding session. Clones are cheap; each session
/// may override the defaults.
#[derive(Debug, Clone)]
pub struct CodingLimits {
    pub max_iterations: usize,
    pub max_tool_calls: usize,
    pub max_model_calls: usize,
    /// Model calls reserved for the final synthesis step (see
    /// [`RESERVED_SYNTHESIS_CALLS`]).
    pub reserved_synthesis_calls: usize,
    pub timeout_ms: u64,
    /// Per-command PTY timeout in seconds (verification commands).
    pub command_timeout_secs: u64,
    pub max_output_bytes: usize,
    pub max_tool_result_chars: usize,
    /// Maximum failed-verification revision attempts (see
    /// [`MAX_REVISION_ATTEMPTS`]).
    pub max_revision_attempts: usize,
    /// When true, `propose_change` on a file NOT named by the plan is denied
    /// instead of being recorded as an unplanned change. Defaults to false:
    /// plan adherence is enforced by recording and surfacing deviations.
    pub strict_plan_adherence: bool,
}

impl Default for CodingLimits {
    fn default() -> Self {
        CodingLimits {
            max_iterations: MAX_CODING_ITERATIONS,
            max_tool_calls: MAX_CODING_TOOL_CALLS,
            max_model_calls: MAX_CODING_MODEL_CALLS,
            reserved_synthesis_calls: RESERVED_SYNTHESIS_CALLS,
            timeout_ms: CODING_TIMEOUT_MS,
            command_timeout_secs: COMMAND_TIMEOUT_SECS,
            max_output_bytes: MAX_CODING_OUTPUT_BYTES,
            max_tool_result_chars: MAX_TOOL_RESULT_CHARS,
            max_revision_attempts: MAX_REVISION_ATTEMPTS,
            strict_plan_adherence: false,
        }
    }
}

impl CodingLimits {
    /// A tiny budget for tests that want to prove limit enforcement quickly.
    pub fn tiny() -> Self {
        CodingLimits {
            max_iterations: 1,
            max_tool_calls: 1,
            max_model_calls: 1,
            reserved_synthesis_calls: RESERVED_SYNTHESIS_CALLS,
            timeout_ms: 500,
            command_timeout_secs: 1,
            max_output_bytes: 256,
            max_tool_result_chars: 64,
            max_revision_attempts: 1,
            strict_plan_adherence: false,
        }
    }

    /// The number of model calls available for execution before the loop is
    /// forced to switch to the final synthesis call. Never exceeds
    /// `max_model_calls`.
    pub fn evidence_model_budget(&self) -> usize {
        self.max_model_calls
            .saturating_sub(self.reserved_synthesis_calls)
    }

    /// Human-readable description of the tools available to coding.
    pub fn describe_tools(&self) -> Vec<String> {
        CODING_ALLOWED_TOOLS
            .iter()
            .map(|name| match *name {
                "list_files" => "list_files — list files in a directory (args: path)".to_string(),
                "read_file" => "read_file — read the contents of a file (args: path)".to_string(),
                "git_status" => "git_status — show git working tree status".to_string(),
                "git_diff" => "git_diff — show git diff of changes".to_string(),
                "propose_change" => "propose_change — propose AND apply one targeted change to one file. Args (JSON): {\"path\": \"relative/file.rs\", \"old\": \"exact text currently in the file (must match uniquely)\", \"new\": \"replacement text\"}. To CREATE a new file pass old=\"\" and the full content as new. The change is prepared against the CURRENT file content, applied immediately, and returns a diff preview. Never passes raw file writes: this tool is the ONLY mutation surface.".to_string(),
                "verify" => "verify — run ONE validation command permitted by the Testing command policy and observe the authoritative exit code. Args (JSON): {\"command\": \"cargo test\"}. exit 0 is success; any non-zero exit code is failure regardless of the output text. Use it to validate the changes you applied before finishing.".to_string(),
                _ => name.to_string(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_limits_are_bounded_and_independent() {
        let limits = CodingLimits::default();
        // Coding is strictly bounded relative to the main loop and stays
        // independent of the Research/Testing/Planning budgets.
        assert!(limits.max_iterations <= 6);
        assert!(limits.max_tool_calls < 100);
        assert!(limits.max_model_calls < 15);
        assert!(limits.timeout_ms <= 30_000);
        assert!(limits.command_timeout_secs < 300);
        // The revision budget is small and well-defined.
        assert!(limits.max_revision_attempts >= 1);
        assert!(limits.max_revision_attempts <= 3);
        // The synthesis reservation always leaves at least one execution call
        // available and never exceeds the model budget.
        assert!(limits.reserved_synthesis_calls >= 1);
        assert!(limits.reserved_synthesis_calls <= limits.max_model_calls);
        assert!(limits.evidence_model_budget() < limits.max_model_calls);
        assert!(
            limits.evidence_model_budget() + limits.reserved_synthesis_calls
                == limits.max_model_calls
        );
        // The per-observation cap stays bounded.
        assert!(limits.max_tool_result_chars <= 8 * 1024);
    }

    #[test]
    fn test_evidence_budget_is_bounded() {
        let limits = CodingLimits {
            max_model_calls: 1,
            ..CodingLimits::default()
        };
        assert_eq!(limits.evidence_model_budget(), 0);
        let limits = CodingLimits {
            max_model_calls: 6,
            reserved_synthesis_calls: 1,
            ..CodingLimits::default()
        };
        assert_eq!(limits.evidence_model_budget(), 5);
    }

    #[test]
    fn test_allowlist_is_explicit_and_narrow() {
        // The allowlist is a fixed set: read-only tools plus the two
        // runtime-intercepted surfaces. Raw filesystem mutation tools and
        // arbitrary command execution are never allowed.
        for tool in CODING_ALLOWED_TOOLS {
            assert!(!tool.contains("create"));
            assert!(!tool.contains("edit"));
            assert!(!tool.contains("run_command"));
        }
        assert!(CODING_ALLOWED_TOOLS.contains(&"list_files"));
        assert!(CODING_ALLOWED_TOOLS.contains(&"read_file"));
        assert!(CODING_ALLOWED_TOOLS.contains(&"git_status"));
        assert!(CODING_ALLOWED_TOOLS.contains(&"git_diff"));
        assert!(CODING_ALLOWED_TOOLS.contains(&"propose_change"));
        assert!(CODING_ALLOWED_TOOLS.contains(&"verify"));
        assert_eq!(CODING_ALLOWED_TOOLS.len(), 6);
    }
}
