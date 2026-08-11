//! Bounded testing limits (Sprint 30D).
//!
//! The autonomous Testing subagent gets its own explicit budget, strictly
//! independent of the main agent's. It never inherits the main ReAct loop's
//! unbounded budget, and it stays independent of the Research subagent's
//! budget too.

/// Maximum number of testing reasoning iterations before giving up.
///
/// Mirrors the Research budget: 6 lets a real LLM run a few validation
/// commands (cargo check → cargo test → targeted test) and still leave room
/// for the reserved synthesis call, while remaining strictly bounded.
pub const MAX_TESTING_ITERATIONS: usize = 6;

/// Maximum total tool calls across the entire testing session.
///
/// Testing executes real commands, each of which may take seconds, so the
/// tool budget is smaller than Research's 20: 12 keeps a session tight while
/// still permitting the multi-step `check → test → targeted test` flow.
pub const MAX_TESTING_TOOL_CALLS: usize = 12;

/// Maximum total model (provider) calls across the testing session.
pub const MAX_TESTING_MODEL_CALLS: usize = 6;

/// Model calls reserved for the final synthesis step.
///
/// The testing loop divides its model budget into an evidence-gathering phase
/// (up to `max_model_calls - reserved_synthesis_calls`) and a guaranteed final
/// synthesis call, so a model that keeps running commands can never starve the
/// final report. The overall budget stays strictly bounded.
pub const RESERVED_SYNTHESIS_CALLS: usize = 1;

/// Hard wall-clock timeout for a testing session, in milliseconds.
///
/// Matches the main task default (`DEFAULT_TASK_TIMEOUT_MS` = 30s) and the
/// Research budget. The session deadline is authoritative: a hanging `cargo
/// test` cannot hang the main CodeBro task.
pub const TESTING_TIMEOUT_MS: u64 = 30_000;

/// Per-command PTY timeout, in seconds.
///
/// Each validation command runs under the existing PTY timeout mechanism
/// (`RunCommand.with_timeout`), so a single hung command terminates before the
/// session deadline. 60s gives a real `cargo test` room to compile the target
/// crate while still being far below the shell default of 300s.
pub const COMMAND_TIMEOUT_SECS: u64 = 60;

/// Maximum accumulated output retained across the whole testing session (in
/// bytes). Testing must stay cheap; oversized command output is truncated.
pub const MAX_TESTING_OUTPUT_BYTES: usize = 16 * 1024;

/// Maximum characters retained from a single command's output.
///
/// Command output is the model's observation surface; it is capped so a chatty
/// `cargo test` cannot blow up the context. The cap never affects the exit
/// code — the machine fact stays authoritative.
pub const MAX_COMMAND_OUTPUT_CHARS: usize = 8 * 1024;

/// The tool names the Testing subagent is explicitly allowed to call.
///
/// Testing is NOT Research: it additionally exposes `run_command`, but that
/// capability is bounded by the explicit [`super::policy::TestingCommandPolicy`]
/// enforced both at the permission layer and in the restricted registry.
pub const TESTING_ALLOWED_TOOLS: &[&str] = &[
    "list_files",
    "read_file",
    "git_status",
    "git_diff",
    "run_command",
];

/// Testing limits applied to one testing session. Clones are cheap; each
/// session may override the defaults.
#[derive(Debug, Clone)]
pub struct TestingLimits {
    pub max_iterations: usize,
    pub max_tool_calls: usize,
    pub max_model_calls: usize,
    /// Model calls reserved for the final synthesis step (see
    /// [`RESERVED_SYNTHESIS_CALLS`]).
    pub reserved_synthesis_calls: usize,
    /// Wall-clock session deadline in milliseconds.
    pub timeout_ms: u64,
    /// Per-command PTY timeout in seconds.
    pub command_timeout_secs: u64,
    /// Maximum accumulated output bytes retained across the session.
    pub max_output_bytes: usize,
    /// Maximum characters retained from a single command's output.
    pub max_command_output_chars: usize,
}

impl Default for TestingLimits {
    fn default() -> Self {
        TestingLimits {
            max_iterations: MAX_TESTING_ITERATIONS,
            max_tool_calls: MAX_TESTING_TOOL_CALLS,
            max_model_calls: MAX_TESTING_MODEL_CALLS,
            reserved_synthesis_calls: RESERVED_SYNTHESIS_CALLS,
            timeout_ms: TESTING_TIMEOUT_MS,
            command_timeout_secs: COMMAND_TIMEOUT_SECS,
            max_output_bytes: MAX_TESTING_OUTPUT_BYTES,
            max_command_output_chars: MAX_COMMAND_OUTPUT_CHARS,
        }
    }
}

impl TestingLimits {
    /// A tiny budget for tests that want to prove limit enforcement quickly.
    pub fn tiny() -> Self {
        TestingLimits {
            max_iterations: 1,
            max_tool_calls: 1,
            max_model_calls: 1,
            reserved_synthesis_calls: RESERVED_SYNTHESIS_CALLS,
            timeout_ms: 500,
            command_timeout_secs: 1,
            max_output_bytes: 256,
            max_command_output_chars: 64,
        }
    }

    /// The number of model calls available for evidence gathering before the
    /// loop is forced to switch to the final synthesis call. Never exceeds
    /// `max_model_calls`.
    pub fn evidence_model_budget(&self) -> usize {
        self.max_model_calls
            .saturating_sub(self.reserved_synthesis_calls)
    }

    /// Human-readable description of the tools available to testing.
    pub fn describe_tools(&self) -> Vec<String> {
        TESTING_ALLOWED_TOOLS
            .iter()
            .map(|name| match *name {
                "list_files" => "list_files — list files in a directory (args: path)".to_string(),
                "read_file" => "read_file — read the contents of a file (args: path)".to_string(),
                "git_status" => "git_status — show git working tree status".to_string(),
                "git_diff" => "git_diff — show git diff of changes".to_string(),
                "run_command" => "run_command — run a validation command allowed by the Testing policy (args: command string). Only read-only build/test/lint/format/git commands are permitted. Mutation commands, shell metacharacters and arbitrary binaries are denied.".to_string(),
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
        let limits = TestingLimits::default();
        // Testing is strictly bounded relative to the main loop and Research:
        // tool calls (12 < 100) and model calls (6 < 15) are strictly smaller.
        assert!(limits.max_iterations <= 6);
        assert!(limits.max_tool_calls < 100);
        assert!(limits.max_model_calls < 15);
        assert!(limits.timeout_ms <= 30_000);
        // The per-command timeout is below the shell default of 300s.
        assert!(limits.command_timeout_secs < 300);
        // Command output stays below the PTY cap of 32 KiB.
        assert!(limits.max_command_output_chars <= 8 * 1024);
        // The synthesis reservation always leaves at least one evidence call
        // available and never exceeds the model budget.
        assert!(limits.reserved_synthesis_calls >= 1);
        assert!(limits.reserved_synthesis_calls <= limits.max_model_calls);
        assert!(limits.evidence_model_budget() < limits.max_model_calls);
        assert!(
            limits.evidence_model_budget() + limits.reserved_synthesis_calls
                == limits.max_model_calls
        );
    }

    #[test]
    fn test_evidence_budget_is_bounded() {
        let limits = TestingLimits {
            max_model_calls: 1,
            ..TestingLimits::default()
        };
        assert_eq!(limits.evidence_model_budget(), 0);
        let limits = TestingLimits {
            max_model_calls: 6,
            reserved_synthesis_calls: 1,
            ..TestingLimits::default()
        };
        assert_eq!(limits.evidence_model_budget(), 5);
    }

    #[test]
    fn test_allowlist_is_explicit() {
        // The allowlist is a fixed set: read-only tools plus a single bounded
        // `run_command`. Mutating tools (create_file / edit_file) are never
        // allowed.
        for tool in TESTING_ALLOWED_TOOLS {
            assert!(!tool.contains("create"));
            assert!(!tool.contains("edit"));
        }
        assert!(TESTING_ALLOWED_TOOLS.contains(&"list_files"));
        assert!(TESTING_ALLOWED_TOOLS.contains(&"read_file"));
        assert!(TESTING_ALLOWED_TOOLS.contains(&"git_status"));
        assert!(TESTING_ALLOWED_TOOLS.contains(&"run_command"));
        // The bounded execution surface is exactly one tool.
        assert_eq!(
            TESTING_ALLOWED_TOOLS
                .iter()
                .filter(|t| **t == "run_command")
                .count(),
            1
        );
    }
}
