//! The Research request/result contract (Sprint 30C).
//!
//! These are the structured types exchanged between the coordinator and the
//! autonomous Research subagent. The result is deliberately structured (not a
//! single giant string) so the parent runtime can consume findings, inspected
//! files and symbols programmatically, while a human-readable rendering is
//! preserved for the EngineeringContext / PromptBuilder.

use std::fmt;
use std::path::PathBuf;

use crate::agent::grounding::GroundedContext;

/// The bounds applied to one research session.
use super::limits::ResearchLimits;

/// How the research session terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResearchTermination {
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

impl fmt::Display for ResearchTermination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResearchTermination::Completed => write!(f, "completed"),
            ResearchTermination::IterationLimit => write!(f, "iteration_limit"),
            ResearchTermination::ToolLimit => write!(f, "tool_limit"),
            ResearchTermination::ModelLimit => write!(f, "model_limit"),
            ResearchTermination::Timeout => write!(f, "timeout"),
            ResearchTermination::Cancelled => write!(f, "cancelled"),
            ResearchTermination::Error => write!(f, "error"),
        }
    }
}

impl ResearchTermination {
    /// Whether the session produced a usable result.
    pub fn is_completed(&self) -> bool {
        matches!(self, ResearchTermination::Completed)
    }
}

/// A request to the autonomous Research subagent.
#[derive(Debug, Clone)]
pub struct ResearchRequest {
    /// The research objective.
    pub task: String,
    /// Absolute workspace root the subagent inspects.
    pub workspace_root: PathBuf,
    /// Sprint 30B grounded context used as the subagent's initial knowledge.
    pub grounding: GroundedContext,
    /// Explicit session bounds.
    pub limits: ResearchLimits,
}

impl ResearchRequest {
    pub fn new(task: impl Into<String>, workspace_root: impl Into<PathBuf>) -> Self {
        ResearchRequest {
            task: task.into(),
            workspace_root: workspace_root.into(),
            grounding: GroundedContext::default(),
            limits: ResearchLimits::default(),
        }
    }

    pub fn with_grounding(mut self, grounding: GroundedContext) -> Self {
        self.grounding = grounding;
        self
    }

    pub fn with_limits(mut self, limits: ResearchLimits) -> Self {
        self.limits = limits;
        self
    }
}

/// Cap a string to a maximum number of characters, preserving the head.
pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect::<String>() + "…"
    }
}

/// One evidence-backed finding returned by research.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchFinding {
    /// A concise statement of the finding.
    pub statement: String,
    /// The file the finding is anchored to, when available.
    pub file: Option<PathBuf>,
    /// The relevant symbol, when available.
    pub symbol: Option<String>,
    /// The tool observation that backs the finding.
    pub evidence: String,
}

/// One real tool observation performed during research.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolObservation {
    pub name: String,
    pub arguments: String,
    pub result: String,
    pub success: bool,
}

/// The structured result of one research session.
#[derive(Debug, Clone)]
pub struct ResearchResult {
    /// Human-readable executive summary.
    pub summary: String,
    /// Evidence-backed findings.
    pub findings: Vec<ResearchFinding>,
    /// Files actually inspected via tools.
    pub files_inspected: Vec<PathBuf>,
    /// Symbols surfaced by grounding or observed file contents.
    pub symbols_found: Vec<String>,
    /// Total real tool calls executed.
    pub tool_calls: usize,
    /// Number of reasoning iterations.
    pub iterations: usize,
    /// Number of model (provider) calls.
    pub model_calls: usize,
    /// Why the session terminated.
    pub termination: ResearchTermination,
    /// Ordered tool observations (evidence trail).
    pub tool_observations: Vec<ToolObservation>,
    /// Explicit limitations of this research pass.
    pub limitations: Vec<String>,
    /// Wall-clock duration.
    pub duration_ms: u64,
    /// Approximate output size in bytes.
    pub output_size: usize,
    /// Provider that executed the research.
    pub provider: String,
    /// Model used.
    pub model: String,
}

impl ResearchResult {
    /// A result for a session that never executed (e.g. provider failure).
    pub fn failed(task: &str, termination: ResearchTermination, error: &str) -> Self {
        ResearchResult {
            summary: format!("Research for '{task}' did not complete: {error}"),
            findings: Vec::new(),
            files_inspected: Vec::new(),
            symbols_found: Vec::new(),
            tool_calls: 0,
            iterations: 0,
            model_calls: 0,
            termination,
            tool_observations: Vec::new(),
            limitations: vec![error.to_string()],
            duration_ms: 0,
            output_size: 0,
            provider: String::new(),
            model: String::new(),
        }
    }

    /// Human-readable rendering used for the `research` ContextFragment that
    /// reaches the main LLM prompt.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "## Autonomous Research Findings\n\nTermination: {}\n",
            self.termination
        ));
        out.push_str(&format!(
            "Iterations: {} | tool calls: {} | model calls: {}\n",
            self.iterations, self.tool_calls, self.model_calls
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
        if !self.symbols_found.is_empty() {
            out.push_str(&format!(
                "Symbols found: {}\n",
                self.symbols_found.join(", ")
            ));
        }
        out.push('\n');
        for finding in &self.findings {
            out.push_str(&format!("- {}\n", finding.statement));
            if let Some(file) = &finding.file {
                out.push_str(&format!("  file: {}\n", file.display()));
            }
            if let Some(symbol) = &finding.symbol {
                out.push_str(&format!("  symbol: {}\n", symbol));
            }
            if !finding.evidence.is_empty() {
                out.push_str(&format!("  evidence: {}\n", finding.evidence));
            }
        }
        if self.findings.is_empty() {
            out.push_str("(no structured findings extracted)\n");
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
            "[research] provider={} model={} iterations={} tool_calls={} model_calls={} files={} symbols={} termination={} duration={}ms output={}B",
            if self.provider.is_empty() { "-" } else { &self.provider },
            if self.model.is_empty() { "-" } else { &self.model },
            self.iterations,
            self.tool_calls,
            self.model_calls,
            self.files_inspected.len(),
            self.symbols_found.len(),
            self.termination,
            self.duration_ms,
            self.output_size,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_termination_display_values() {
        assert_eq!(ResearchTermination::Completed.to_string(), "completed");
        assert_eq!(
            ResearchTermination::IterationLimit.to_string(),
            "iteration_limit"
        );
        assert_eq!(ResearchTermination::ToolLimit.to_string(), "tool_limit");
        assert_eq!(ResearchTermination::ModelLimit.to_string(), "model_limit");
        assert_eq!(ResearchTermination::Timeout.to_string(), "timeout");
        assert_eq!(ResearchTermination::Cancelled.to_string(), "cancelled");
        assert_eq!(ResearchTermination::Error.to_string(), "error");
    }

    #[test]
    fn test_failed_result_is_bounded_error_result() {
        let result = ResearchResult::failed(
            "trace the loop",
            ResearchTermination::Error,
            "provider unavailable",
        );
        assert_eq!(result.termination, ResearchTermination::Error);
        assert!(result.summary.contains("provider unavailable"));
        assert!(result.findings.is_empty());
        assert_eq!(result.tool_calls, 0);
    }

    #[test]
    fn test_render_includes_key_sections() {
        let result = ResearchResult::failed(
            "trace the loop",
            ResearchTermination::Cancelled,
            "cancelled",
        );
        let rendered = result.render();
        assert!(rendered.contains("Autonomous Research Findings"));
        assert!(rendered.contains("Termination:"));
        assert!(rendered.contains("cancelled"));
    }
}
