//! Bounded planning limits (Sprint 30E).
//!
//! The autonomous Planning subagent gets its own explicit budget, strictly
//! independent of the main agent's and of the Research/Testing budgets.
//! Planning is evidence-driven and read-only; it should perform a handful of
//! targeted reads at most, so the tool budget is deliberately conservative.

/// Maximum number of planning reasoning iterations before giving up.
///
/// Mirrors the Research/Testing budgets. 6 lets a real LLM verify a couple of
/// research claims with targeted reads and still leave room for the reserved
/// final synthesis call.
pub const MAX_PLANNING_ITERATIONS: usize = 6;

/// Maximum total tool calls across the entire planning session.
///
/// Planning must VERIFY, not rediscover: it starts from Research/Testing
/// evidence and only performs targeted reads when more evidence is required.
/// A session that burns 12 tool calls on broad scans has stopped planning.
pub const MAX_PLANNING_TOOL_CALLS: usize = 12;

/// Maximum total model (provider) calls across the planning session.
pub const MAX_PLANNING_MODEL_CALLS: usize = 6;

/// Model calls reserved for the final synthesis step.
///
/// The planning loop divides its model budget into an evidence phase (up to
/// `max_model_calls - reserved_synthesis_calls`) and a guaranteed final
/// implementation-plan synthesis call. A model that keeps reading can never
/// starve the plan.
pub const RESERVED_SYNTHESIS_CALLS: usize = 1;

/// Hard wall-clock timeout for a planning session, in milliseconds.
///
/// Matches the main task default and the Research/Testing sessions. Planning
/// is strictly read-only and cheap; 30s is generous for a few targeted reads
/// plus a synthesis.
pub const PLANNING_TIMEOUT_MS: u64 = 30_000;

/// Maximum accumulated output retained across the whole planning session (in
/// bytes). Planning stays small — it consumes (not re-runs) evidence.
pub const MAX_PLANNING_OUTPUT_BYTES: usize = 16 * 1024;

/// Maximum characters retained from a single tool result.
///
/// Planning verifies claims, so a single read may legitimately be a large
/// file; 8 KiB of context per observation is ample without blowing up the
/// session.
pub const MAX_TOOL_RESULT_CHARS: usize = 8 * 1024;

/// The tool names the Planning subagent is explicitly allowed to call.
///
/// Planning is strictly READ-ONLY. It reasons from Research/Testing evidence
/// and performs targeted reads only. `run_command` is NOT allowed — Testing
/// owns command execution; Planning never executes anything.
pub const PLANNING_ALLOWED_TOOLS: &[&str] = &["list_files", "read_file", "git_status", "git_diff"];

/// Planning limits applied to one planning session. Clones are cheap; each
/// session may override the defaults.
#[derive(Debug, Clone)]
pub struct PlanningLimits {
    pub max_iterations: usize,
    pub max_tool_calls: usize,
    pub max_model_calls: usize,
    /// Model calls reserved for the final synthesis step (see
    /// [`RESERVED_SYNTHESIS_CALLS`]).
    pub reserved_synthesis_calls: usize,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub max_tool_result_chars: usize,
}

impl Default for PlanningLimits {
    fn default() -> Self {
        PlanningLimits {
            max_iterations: MAX_PLANNING_ITERATIONS,
            max_tool_calls: MAX_PLANNING_TOOL_CALLS,
            max_model_calls: MAX_PLANNING_MODEL_CALLS,
            reserved_synthesis_calls: RESERVED_SYNTHESIS_CALLS,
            timeout_ms: PLANNING_TIMEOUT_MS,
            max_output_bytes: MAX_PLANNING_OUTPUT_BYTES,
            max_tool_result_chars: MAX_TOOL_RESULT_CHARS,
        }
    }
}

impl PlanningLimits {
    /// A tiny budget for tests that want to prove limit enforcement quickly.
    pub fn tiny() -> Self {
        PlanningLimits {
            max_iterations: 1,
            max_tool_calls: 1,
            max_model_calls: 1,
            reserved_synthesis_calls: RESERVED_SYNTHESIS_CALLS,
            timeout_ms: 500,
            max_output_bytes: 256,
            max_tool_result_chars: 64,
        }
    }

    /// The number of model calls available for evidence gathering before the
    /// loop is forced to switch to the final synthesis call. Never exceeds
    /// `max_model_calls`.
    pub fn evidence_model_budget(&self) -> usize {
        self.max_model_calls
            .saturating_sub(self.reserved_synthesis_calls)
    }

    /// Human-readable description of the tools available to planning.
    pub fn describe_tools(&self) -> Vec<String> {
        PLANNING_ALLOWED_TOOLS
            .iter()
            .map(|name| match *name {
                "list_files" => "list_files — list files in a directory (args: path)".to_string(),
                "read_file" => "read_file — read the contents of a file (args: path)".to_string(),
                "git_status" => "git_status — show git working tree status".to_string(),
                "git_diff" => "git_diff — show git diff of changes".to_string(),
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
        let limits = PlanningLimits::default();
        // Planning is strictly bounded relative to the main loop and stays
        // independent of the Research/Testing budgets.
        assert!(limits.max_iterations <= 6);
        assert!(limits.max_tool_calls < 100);
        assert!(limits.max_model_calls < 15);
        assert!(limits.timeout_ms <= 30_000);
        // The synthesis reservation always leaves at least one evidence call
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
        let limits = PlanningLimits {
            max_model_calls: 1,
            ..PlanningLimits::default()
        };
        assert_eq!(limits.evidence_model_budget(), 0);
        let limits = PlanningLimits {
            max_model_calls: 6,
            reserved_synthesis_calls: 1,
            ..PlanningLimits::default()
        };
        assert_eq!(limits.evidence_model_budget(), 5);
    }

    #[test]
    fn test_allowlist_is_explicit_and_strictly_read_only() {
        // The allowlist is a fixed set of non-mutating tools. Planning never
        // executes anything: run_command is absent.
        for tool in PLANNING_ALLOWED_TOOLS {
            assert!(!tool.contains("create"));
            assert!(!tool.contains("edit"));
            assert!(!tool.contains("run_command"));
        }
        assert!(PLANNING_ALLOWED_TOOLS.contains(&"list_files"));
        assert!(PLANNING_ALLOWED_TOOLS.contains(&"read_file"));
        assert!(PLANNING_ALLOWED_TOOLS.contains(&"git_status"));
        assert!(PLANNING_ALLOWED_TOOLS.contains(&"git_diff"));
        assert_eq!(PLANNING_ALLOWED_TOOLS.len(), 4);
    }
}
