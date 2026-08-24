//! Review session bounds (Sprint 30G).
//!
//! Review is bounded tighter than Coding — it is read-only inspection, so its
//! model budget can be conservative. The synthesis call is reserved so a
//! model that keeps inspecting can never starve the final report.

/// Maximum number of review reasoning iterations before giving up.
pub const MAX_REVIEW_ITERATIONS: usize = 6;

/// Maximum total tool calls across the entire review session.
pub const MAX_REVIEW_TOOL_CALLS: usize = 20;

/// Maximum total model (provider) calls across the review session.
pub const MAX_REVIEW_MODEL_CALLS: usize = 6;

/// Model calls reserved for the final synthesis step.
pub const RESERVED_REVIEW_SYNTHESIS_CALLS: usize = 1;

/// Hard wall-clock timeout for a review session.
pub const REVIEW_TIMEOUT_MS: u64 = 30_000;

/// Maximum accumulated output retained across the whole review session (in
/// bytes). Oversized diffs and command output are truncated.
pub const MAX_REVIEW_OUTPUT_BYTES: usize = 16 * 1024;

/// Maximum characters retained from a single tool result or finding preview.
pub const MAX_REVIEW_TOOL_RESULT_CHARS: usize = 8 * 1024;

/// The tool names the Review subagent is explicitly allowed to call.
///
/// Review is read-only only: list_files, read_file, git_status, git_diff. No
/// mutation, no command execution, no propose_change, no verify.
pub const REVIEW_ALLOWED_TOOLS: &[&str] = &["list_files", "read_file", "git_status", "git_diff"];

/// Review limits applied to one review session.
#[derive(Debug, Clone)]
pub struct ReviewLimits {
    pub max_iterations: usize,
    pub max_tool_calls: usize,
    pub max_model_calls: usize,
    pub reserved_synthesis_calls: usize,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub max_tool_result_chars: usize,
}

impl Default for ReviewLimits {
    fn default() -> Self {
        ReviewLimits {
            max_iterations: MAX_REVIEW_ITERATIONS,
            max_tool_calls: MAX_REVIEW_TOOL_CALLS,
            max_model_calls: MAX_REVIEW_MODEL_CALLS,
            reserved_synthesis_calls: RESERVED_REVIEW_SYNTHESIS_CALLS,
            timeout_ms: REVIEW_TIMEOUT_MS,
            max_output_bytes: MAX_REVIEW_OUTPUT_BYTES,
            max_tool_result_chars: MAX_REVIEW_TOOL_RESULT_CHARS,
        }
    }
}

impl ReviewLimits {
    /// A tiny budget for tests that want to prove limit enforcement quickly.
    pub fn tiny() -> Self {
        ReviewLimits {
            max_iterations: 1,
            max_tool_calls: 1,
            max_model_calls: 1,
            reserved_synthesis_calls: RESERVED_REVIEW_SYNTHESIS_CALLS,
            timeout_ms: 500,
            max_output_bytes: 256,
            max_tool_result_chars: 64,
        }
    }

    /// The number of model calls available for evidence gathering before the
    /// loop is forced to switch to the final synthesis call.
    pub fn evidence_model_budget(&self) -> usize {
        self.max_model_calls
            .saturating_sub(self.reserved_synthesis_calls)
    }

    /// Human-readable description of the tools available to review.
    pub fn describe_tools(&self) -> Vec<String> {
        REVIEW_ALLOWED_TOOLS
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
    fn test_default_limits_are_bounded() {
        let limits = ReviewLimits::default();
        assert!(limits.max_iterations <= 6);
        assert!(limits.max_tool_calls < 100);
        assert!(limits.max_model_calls <= 6);
        assert!(limits.timeout_ms <= 30_000);
        assert!(limits.reserved_synthesis_calls >= 1);
        assert!(limits.reserved_synthesis_calls <= limits.max_model_calls);
        assert!(limits.evidence_model_budget() < limits.max_model_calls);
    }

    #[test]
    fn test_allowlist_is_explicit_and_read_only() {
        for tool in REVIEW_ALLOWED_TOOLS {
            assert!(!tool.contains("create"));
            assert!(!tool.contains("edit"));
            assert!(!tool.contains("run_command"));
            assert!(!tool.contains("propose"));
            assert!(!tool.contains("verify"));
        }
        assert_eq!(REVIEW_ALLOWED_TOOLS.len(), 4);
        assert!(REVIEW_ALLOWED_TOOLS.contains(&"list_files"));
        assert!(REVIEW_ALLOWED_TOOLS.contains(&"read_file"));
        assert!(REVIEW_ALLOWED_TOOLS.contains(&"git_status"));
        assert!(REVIEW_ALLOWED_TOOLS.contains(&"git_diff"));
    }
}
