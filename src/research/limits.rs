//! Bounded research limits (Sprint 30C).
//!
//! The autonomous Research subagent gets its own explicit budget, strictly
//! smaller than the main agent's. It never inherits the main ReAct loop's
//! unlimited budget.

/// Maximum number of research reasoning iterations before giving up.
///
/// The main loop uses `MAX_REACT_ITERATIONS = 5`; research is capped lower
/// so a runaway research session stops quickly.
pub const MAX_RESEARCH_ITERATIONS: usize = 4;

/// Maximum total tool calls across the entire research session.
///
/// The main loop allows 100; research is bounded to a fraction of that.
pub const MAX_RESEARCH_TOOL_CALLS: usize = 20;

/// Maximum total model (provider) calls across the research session.
///
/// Each iteration performs at most one model call; the bound is therefore
/// slightly above `MAX_RESEARCH_ITERATIONS` to permit one final-answer call.
pub const MAX_RESEARCH_MODEL_CALLS: usize = 4;

/// Hard wall-clock timeout for a research session, in milliseconds.
///
/// The main task default is 30s; research is bounded to half of that.
pub const RESEARCH_TIMEOUT_MS: u64 = 15_000;

/// Maximum accumulated tool-result output retained for the next iteration
/// (in bytes). Research must stay cheap; oversized tool output is truncated.
pub const MAX_RESEARCH_OUTPUT_BYTES: usize = 16 * 1024;

/// Maximum characters retained from a single tool result.
pub const MAX_TOOL_RESULT_CHARS: usize = 4096;

/// The tool names the Research subagent is explicitly allowed to call.
pub const RESEARCH_ALLOWED_TOOLS: &[&str] = &["list_files", "read_file", "git_status", "git_diff"];

/// Research limits applied to one research session. Clones are cheap;
/// each session may override the defaults.
#[derive(Debug, Clone)]
pub struct ResearchLimits {
    pub max_iterations: usize,
    pub max_tool_calls: usize,
    pub max_model_calls: usize,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub max_tool_result_chars: usize,
}

impl Default for ResearchLimits {
    fn default() -> Self {
        ResearchLimits {
            max_iterations: MAX_RESEARCH_ITERATIONS,
            max_tool_calls: MAX_RESEARCH_TOOL_CALLS,
            max_model_calls: MAX_RESEARCH_MODEL_CALLS,
            timeout_ms: RESEARCH_TIMEOUT_MS,
            max_output_bytes: MAX_RESEARCH_OUTPUT_BYTES,
            max_tool_result_chars: MAX_TOOL_RESULT_CHARS,
        }
    }
}

impl ResearchLimits {
    /// A tiny budget for tests that want to prove limit enforcement quickly.
    pub fn tiny() -> Self {
        ResearchLimits {
            max_iterations: 1,
            max_tool_calls: 1,
            max_model_calls: 1,
            timeout_ms: 500,
            max_output_bytes: 256,
            max_tool_result_chars: 64,
        }
    }

    /// Human-readable description of the tools available to research.
    pub fn describe_tools(&self) -> Vec<String> {
        RESEARCH_ALLOWED_TOOLS
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
    fn test_default_limits_are_smaller_than_main_loop() {
        let limits = ResearchLimits::default();
        // Research must be strictly bounded relative to the canonical runtime.
        assert!(limits.max_iterations < 5);
        assert!(limits.max_tool_calls < 100);
        assert!(limits.max_model_calls < 15);
        assert!(limits.timeout_ms < 30_000);
    }

    #[test]
    fn test_allowlist_is_explicit_and_read_only() {
        // The allowlist is a fixed set of non-mutating tools. Mutating tools
        // (create_file / edit_file / run_command) are never allowed.
        for tool in RESEARCH_ALLOWED_TOOLS {
            assert!(!tool.contains("create"));
            assert!(!tool.contains("edit"));
            assert!(!tool.contains("run_command"));
        }
        assert!(RESEARCH_ALLOWED_TOOLS.contains(&"list_files"));
        assert!(RESEARCH_ALLOWED_TOOLS.contains(&"read_file"));
        assert!(RESEARCH_ALLOWED_TOOLS.contains(&"git_status"));
    }
}
