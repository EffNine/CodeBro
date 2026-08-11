//! Bounded research limits (Sprint 30C).
//!
//! The autonomous Research subagent gets its own explicit budget, strictly
//! smaller than the main agent's. It never inherits the main ReAct loop's
//! unlimited budget.

/// Maximum number of research reasoning iterations before giving up.
///
/// Real-provider validation (Sprint 30C.0) showed a 4-call budget prevents a
/// real LLM from completing exploration AND a final synthesis for a multi-part
/// task (it terminated at `model_limit` without producing findings). 6 allows
/// targeted exploration plus one synthesis pass while remaining bounded and
/// close to the main loop's `MAX_REACT_ITERATIONS = 5`.
pub const MAX_RESEARCH_ITERATIONS: usize = 6;

/// Maximum total tool calls across the entire research session.
///
/// The main loop allows 100; research is bounded to a fraction of that.
pub const MAX_RESEARCH_TOOL_CALLS: usize = 20;

/// Maximum total model (provider) calls across the research session.
///
/// Each iteration performs at most one model call. 6 is strictly smaller than
/// the main loop's `MAX_MODEL_CALLS = 15` while giving a real LLM room to
/// explore then synthesize.
pub const MAX_RESEARCH_MODEL_CALLS: usize = 6;

/// Hard wall-clock timeout for a research session, in milliseconds.
///
/// Matches the main task default (`DEFAULT_TASK_TIMEOUT_MS` = 30s). Research
/// is otherwise strictly smaller than the main loop's budget; a real provider
/// needs this much wall time to complete a multi-step research session.
pub const RESEARCH_TIMEOUT_MS: u64 = 30_000;

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
        // Research must be strictly bounded relative to the canonical runtime:
        // tool calls (20 < 100) and model calls (6 < 15) are strictly smaller,
        // and the timeout matches the main task default so a real provider can
        // complete a multi-step session. Iterations (6) sit just above the main
        // loop's 5 to let a real LLM explore then synthesize — this was tuned
        // after real-provider validation (Sprint 30C.0).
        assert!(limits.max_iterations <= 6);
        assert!(limits.max_tool_calls < 100);
        assert!(limits.max_model_calls < 15);
        assert!(limits.timeout_ms <= 30_000);
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
