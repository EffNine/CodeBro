//! The Planning request/result contract (Sprint 30E).
//!
//! These are the structured types exchanged between the coordinator and the
//! autonomous Planning subagent. Planning consumes the evidence of Research
//! (files, symbols, findings) and Testing (commands, exit codes, failures)
//! plus the GroundedContext, and produces an evidence-backed implementation
//! plan.
//!
//! The contract deliberately separates:
//! - machine facts (research observations, test exit codes) from
//! - model reasoning (rationale, priorities, assumptions)
//!
//! Every `PlanStep` carries the file/symbol targets, the validation the future
//! Coding/Testing stages should run, and the evidence that supports it. Plan
//! steps are concrete. A weak "analyze the code" step is not a plan.

use std::fmt;
use std::path::PathBuf;

use crate::agent::grounding::GroundedContext;
use crate::research::{ResearchResult, ToolObservation};
use crate::testing::TestingResult;

use super::limits::PlanningLimits;

/// How the planning session terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningTermination {
    /// The subagent produced a final implementation plan.
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

impl fmt::Display for PlanningTermination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanningTermination::Completed => write!(f, "completed"),
            PlanningTermination::IterationLimit => write!(f, "iteration_limit"),
            PlanningTermination::ToolLimit => write!(f, "tool_limit"),
            PlanningTermination::ModelLimit => write!(f, "model_limit"),
            PlanningTermination::Timeout => write!(f, "timeout"),
            PlanningTermination::Cancelled => write!(f, "cancelled"),
            PlanningTermination::Error => write!(f, "error"),
        }
    }
}

impl PlanningTermination {
    /// Whether the session produced a usable result.
    pub fn is_completed(&self) -> bool {
        matches!(self, PlanningTermination::Completed)
    }
}

/// A request to the autonomous Planning subagent.
#[derive(Debug, Clone)]
pub struct PlanningRequest {
    /// The planning objective.
    pub task: String,
    /// Absolute workspace root the subagent inspects (read-only).
    pub workspace_root: PathBuf,
    /// Sprint 30B grounded context used as the subagent's initial knowledge.
    pub grounding: GroundedContext,
    /// Optional Sprint 30C research evidence.
    pub research: Option<ResearchResult>,
    /// Optional Sprint 30D testing evidence (authoritative machine facts).
    pub testing: Option<TestingResult>,
    /// Explicit session bounds.
    pub limits: PlanningLimits,
}

impl PlanningRequest {
    pub fn new(task: impl Into<String>, workspace_root: impl Into<PathBuf>) -> Self {
        PlanningRequest {
            task: task.into(),
            workspace_root: workspace_root.into(),
            grounding: GroundedContext::default(),
            research: None,
            testing: None,
            limits: PlanningLimits::default(),
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

    pub fn with_limits(mut self, limits: PlanningLimits) -> Self {
        self.limits = limits;
        self
    }
}

/// One concrete implementation step of the plan.
///
/// A useful step names the file and symbol to change, explains why the change
/// is needed, how it should be validated, and which evidence supports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    /// Step position in the plan (1-based).
    pub order: usize,
    /// The concrete action, e.g. "modify run_execution_loop". Never a vague
    /// "analyze the code" — planning owns strategy, Coding owns patches.
    pub action: String,
    /// Files the step must change.
    pub target_files: Vec<PathBuf>,
    /// Symbols the step targets.
    pub target_symbols: Vec<String>,
    /// Why this change is required, grounded in the objective.
    pub rationale: String,
    /// Dependencies or coupling the step relies on (discovered from evidence).
    pub dependencies: Vec<String>,
    /// Concrete validation commands the Coding/Testing stage should run.
    pub validation: Vec<String>,
    /// Potential regression point of this step.
    pub risk: String,
    /// Evidence provenance backing this step (e.g. "research: src/x.rs",
    /// "testing: exit_code 0", "planning_read: ...").
    pub evidence: Vec<String>,
}

/// A potential regression point of the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRisk {
    pub description: String,
    pub severity: String,
    pub mitigation: String,
}

/// One item of evidence provenance used by the planner.
///
/// `source` is one of: `research`, `testing`, `grounding`, `planning_read`.
/// The parent can audit WHICH evidence informed the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningEvidence {
    pub source: String,
    pub reference: String,
    pub summary: String,
}

/// The structured result of one planning session.
#[derive(Debug, Clone)]
pub struct PlanningResult {
    /// Human-readable executive summary: what currently exists, what must
    /// change, and why.
    pub summary: String,
    /// The ordered implementation plan.
    pub plan: Vec<PlanStep>,
    /// Every file the plan touches (union of step targets).
    pub affected_files: Vec<PathBuf>,
    /// Every symbol the plan targets (union of step targets).
    pub affected_symbols: Vec<String>,
    /// Dependencies/coupling the plan relies on.
    pub dependencies: Vec<String>,
    /// Test files the plan says should be updated or new test targets.
    pub tests_to_update: Vec<PathBuf>,
    /// Potential regression points.
    pub risks: Vec<PlanningRisk>,
    /// Anything the planner could not verify (never silently a fact).
    pub assumptions: Vec<String>,
    /// The full evidence-provenance trail.
    pub evidence: Vec<PlanningEvidence>,
    /// Total real tool calls executed (all read-only).
    pub tool_calls: usize,
    /// Number of reasoning iterations.
    pub iterations: usize,
    /// Number of model (provider) calls.
    pub model_calls: usize,
    /// Why the session terminated.
    pub termination: PlanningTermination,
    /// Whether the final plan synthesis was actually produced. `false` means
    /// the session ended before a final plan could be written; the structured
    /// evidence trail is still preserved and a plan is never fabricated.
    pub synthesis_complete: bool,
    /// Ordered real tool observations (evidence trail).
    pub tool_observations: Vec<ToolObservation>,
    /// Explicit limitations of this planning pass (budget, unverified items).
    pub limitations: Vec<String>,
    /// Wall-clock duration.
    pub duration_ms: u64,
    /// Approximate output size in bytes.
    pub output_size: usize,
    /// Provider that executed the planning.
    pub provider: String,
    /// Model used.
    pub model: String,
}

impl PlanningResult {
    /// A result for a session that never executed (e.g. provider failure).
    pub fn failed(task: &str, termination: PlanningTermination, error: &str) -> Self {
        PlanningResult {
            summary: format!("Planning for '{task}' did not complete: {error}"),
            plan: Vec::new(),
            affected_files: Vec::new(),
            affected_symbols: Vec::new(),
            dependencies: Vec::new(),
            tests_to_update: Vec::new(),
            risks: Vec::new(),
            assumptions: Vec::new(),
            evidence: Vec::new(),
            tool_calls: 0,
            iterations: 0,
            model_calls: 0,
            termination,
            synthesis_complete: false,
            tool_observations: Vec::new(),
            limitations: vec![error.to_string()],
            duration_ms: 0,
            output_size: 0,
            provider: String::new(),
            model: String::new(),
        }
    }

    /// Human-readable rendering used for the `planning` ContextFragment that
    /// reaches the main LLM prompt. Preserves the structured plan, the
    /// affected components and the evidence trail.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "## Autonomous Planning\n\nTermination: {}\n",
            self.termination
        ));
        out.push_str(&format!(
            "Iterations: {} | tool calls: {} | model calls: {} | steps: {} | synthesis complete: {}\n",
            self.iterations,
            self.tool_calls,
            self.model_calls,
            self.plan.len(),
            self.synthesis_complete
        ));
        if !self.provider.is_empty() {
            out.push_str(&format!("Provider: {}\n", self.provider));
        }

        out.push_str("\n## Existing implementation\n");
        for line in section_lines(&self.summary, "EXISTING") {
            out.push_str(&line);
        }

        out.push_str("\n## Required change\n");
        for line in section_lines(&self.summary, "REQUIRED CHANGE") {
            out.push_str(&line);
        }

        out.push_str("\n## Affected components\n");
        if self.affected_files.is_empty() {
            out.push_str("(none)\n");
        } else {
            for file in &self.affected_files {
                out.push_str(&format!("- {}\n", file.display()));
            }
        }
        if !self.affected_symbols.is_empty() {
            out.push_str(&format!("Symbols: {}\n", self.affected_symbols.join(", ")));
        }

        out.push_str("\n## Implementation plan\n");
        if self.plan.is_empty() {
            out.push_str("(no concrete steps extracted)\n");
        } else {
            for step in &self.plan {
                out.push_str(&format!("{}. {}\n", step.order, step.action));
                if !step.target_files.is_empty() {
                    out.push_str(&format!(
                        "   Files: {}\n",
                        step.target_files
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !step.target_symbols.is_empty() {
                    out.push_str(&format!("   Symbols: {}\n", step.target_symbols.join(", ")));
                }
                if !step.rationale.is_empty() {
                    out.push_str(&format!("   Reason: {}\n", step.rationale));
                }
                if !step.validation.is_empty() {
                    out.push_str(&format!("   Validation: {}\n", step.validation.join("; ")));
                }
                if !step.risk.is_empty() {
                    out.push_str(&format!("   Risk: {}\n", step.risk));
                }
            }
        }

        if !self.tests_to_update.is_empty() {
            out.push_str("\n## Tests to update\n");
            for path in &self.tests_to_update {
                out.push_str(&format!("- {}\n", path.display()));
            }
        }

        if !self.risks.is_empty() {
            out.push_str("\n## Risks\n");
            for risk in &self.risks {
                out.push_str(&format!(
                    "- {} (severity: {}){}\n",
                    risk.description,
                    risk.severity,
                    if risk.mitigation.is_empty() {
                        String::new()
                    } else {
                        format!(" | mitigation: {}", risk.mitigation)
                    }
                ));
            }
        }

        if !self.dependencies.is_empty() {
            out.push_str(&format!(
                "\n## Dependencies\n{}\n",
                self.dependencies.join(", ")
            ));
        }

        if !self.assumptions.is_empty() {
            out.push_str("\n## Assumptions\n");
            for assumption in &self.assumptions {
                out.push_str(&format!("- {}\n", assumption));
            }
        }

        if !self.evidence.is_empty() {
            out.push_str("\n## Evidence for the plan\n");
            for entry in &self.evidence {
                out.push_str(&format!(
                    "- [{}] {} — {}\n",
                    entry.source, entry.reference, entry.summary
                ));
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
            "[planning] provider={} model={} iterations={} tool_calls={} model_calls={} steps={} files={} symbols={} tests={} risks={} termination={} synthesis={} duration={}ms output={}B",
            if self.provider.is_empty() { "-" } else { &self.provider },
            if self.model.is_empty() { "-" } else { &self.model },
            self.iterations,
            self.tool_calls,
            self.model_calls,
            self.plan.len(),
            self.affected_files.len(),
            self.affected_symbols.len(),
            self.tests_to_update.len(),
            self.risks.len(),
            self.termination,
            self.synthesis_complete,
            self.duration_ms,
            self.output_size,
        )
    }
}

/// Extract the lines of an explicitly-marked summary section, if the model
/// marked sections like `## EXISTING IMPLEMENTATION`.
fn section_lines(summary: &str, section: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut capturing = false;
    for line in summary.lines() {
        let upper = line.to_uppercase().trim().to_string();
        if upper.starts_with("##") || upper.starts_with("#") {
            capturing = upper.contains(section);
            continue;
        }
        if capturing && !line.trim().is_empty() {
            lines.push(format!("{}\n", line));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_termination_display_values() {
        assert_eq!(PlanningTermination::Completed.to_string(), "completed");
        assert_eq!(
            PlanningTermination::IterationLimit.to_string(),
            "iteration_limit"
        );
        assert_eq!(PlanningTermination::ToolLimit.to_string(), "tool_limit");
        assert_eq!(PlanningTermination::ModelLimit.to_string(), "model_limit");
        assert_eq!(PlanningTermination::Timeout.to_string(), "timeout");
        assert_eq!(PlanningTermination::Cancelled.to_string(), "cancelled");
        assert_eq!(PlanningTermination::Error.to_string(), "error");
    }

    #[test]
    fn test_request_builders_default_cleanly() {
        let request = PlanningRequest::new("plan the refactor", "/repo");
        assert_eq!(request.task, "plan the refactor");
        assert_eq!(request.workspace_root, PathBuf::from("/repo"));
        assert!(request.research.is_none());
        assert!(request.testing.is_none());
        assert_eq!(request.limits.max_model_calls, 6);
    }

    #[test]
    fn test_failed_result_is_bounded_error_result() {
        let result = PlanningResult::failed(
            "plan the refactor",
            PlanningTermination::Error,
            "provider unavailable",
        );
        assert_eq!(result.termination, PlanningTermination::Error);
        assert!(result.summary.contains("provider unavailable"));
        assert!(result.plan.is_empty());
        assert!(result.affected_files.is_empty());
        assert_eq!(result.tool_calls, 0);
        assert!(!result.synthesis_complete);
    }

    #[test]
    fn test_render_includes_plan_and_evidence() {
        let result = PlanningResult {
            summary: "src/parser.rs currently owns X and must be split.".to_string(),
            plan: vec![PlanStep {
                order: 1,
                action: "modify parse_tool_calls".to_string(),
                target_files: vec![PathBuf::from("src/parser.rs")],
                target_symbols: vec!["parse_tool_calls".to_string()],
                rationale: "the parser currently owns X".to_string(),
                dependencies: vec!["src/parser.rs".to_string()],
                validation: vec!["cargo test parser_tests".to_string()],
                risk: "callers of parse_tool_calls may break".to_string(),
                evidence: vec!["research: parser.rs".to_string()],
            }],
            affected_files: vec![PathBuf::from("src/parser.rs")],
            affected_symbols: vec!["parse_tool_calls".to_string()],
            dependencies: vec!["src/parser.rs".to_string()],
            tests_to_update: vec![PathBuf::from("src/parser_tests.rs")],
            risks: vec![PlanningRisk {
                description: "callers may break".to_string(),
                severity: "medium".to_string(),
                mitigation: "add regression coverage".to_string(),
            }],
            assumptions: vec!["no external callers".to_string()],
            evidence: vec![PlanningEvidence {
                source: "research".to_string(),
                reference: "src/parser.rs".to_string(),
                summary: "parse_tool_calls lives here".to_string(),
            }],
            tool_calls: 1,
            iterations: 2,
            model_calls: 2,
            termination: PlanningTermination::Completed,
            synthesis_complete: true,
            tool_observations: Vec::new(),
            limitations: Vec::new(),
            duration_ms: 500,
            output_size: 200,
            provider: "mock".to_string(),
            model: "mock-model".to_string(),
        };
        let rendered = result.render();
        assert!(rendered.contains("Autonomous Planning"));
        assert!(rendered.contains("1. modify parse_tool_calls"));
        assert!(rendered.contains("src/parser.rs"));
        assert!(rendered.contains("parse_tool_calls"));
        assert!(rendered.contains("cargo test parser_tests"));
        assert!(rendered.contains("[research]"));
        assert!(rendered.contains("## Affected components"));
        assert!(rendered.contains("## Assumptions"));
    }
}
