//! The Review request/result contract (Sprint 30G).
//!
//! Review consumes the structured results from Research, Testing, Planning
//! and Coding, independently inspects the actual repository state, compares
//! intended vs. actual changes, evaluates verification evidence, and produces
//! a structured `ReviewResult` with evidence-backed findings.

use std::fmt;
use std::path::PathBuf;

use crate::agent::grounding::GroundedContext;
use crate::planning::PlanningResult;
use crate::research::ResearchResult;
use crate::testing::TestingResult;

use super::limits::ReviewLimits;
use crate::coding::CodingResult;

// =========================================================================
// Terminations
// =========================================================================

/// How the review session terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewTermination {
    /// The reviewer produced a final synthesis report.
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

impl fmt::Display for ReviewTermination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewTermination::Completed => write!(f, "completed"),
            ReviewTermination::IterationLimit => write!(f, "iteration_limit"),
            ReviewTermination::ToolLimit => write!(f, "tool_limit"),
            ReviewTermination::ModelLimit => write!(f, "model_limit"),
            ReviewTermination::Timeout => write!(f, "timeout"),
            ReviewTermination::Cancelled => write!(f, "cancelled"),
            ReviewTermination::Error => write!(f, "error"),
        }
    }
}

impl ReviewTermination {
    /// Whether the session produced a usable result.
    pub fn is_completed(&self) -> bool {
        matches!(self, ReviewTermination::Completed)
    }
}

// =========================================================================
// Request
// =========================================================================

/// A request to the autonomous Review subagent.
#[derive(Debug, Clone)]
pub struct ReviewRequest {
    /// The original engineering task being reviewed.
    pub task: String,
    /// Absolute workspace root the reviewer inspects.
    pub workspace_root: PathBuf,
    /// Sprint 30B grounded context used as initial knowledge.
    pub grounding: GroundedContext,
    /// Optional Sprint 30C research evidence.
    pub research: Option<ResearchResult>,
    /// Optional Sprint 30D testing evidence.
    pub testing: Option<TestingResult>,
    /// Optional Sprint 30E planning evidence.
    pub planning: Option<PlanningResult>,
    /// Optional Sprint 30F coding evidence.
    pub coding: Option<CodingResult>,
    /// Explicit session bounds.
    pub limits: ReviewLimits,
}

impl ReviewRequest {
    pub fn new(task: impl Into<String>, workspace_root: impl Into<PathBuf>) -> Self {
        ReviewRequest {
            task: task.into(),
            workspace_root: workspace_root.into(),
            grounding: GroundedContext::default(),
            research: None,
            testing: None,
            planning: None,
            coding: None,
            limits: ReviewLimits::default(),
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

    pub fn with_coding(mut self, coding: Option<CodingResult>) -> Self {
        self.coding = coding;
        self
    }

    pub fn with_limits(mut self, limits: ReviewLimits) -> Self {
        self.limits = limits;
        self
    }
}

// =========================================================================
// Findings
// =========================================================================

/// Severity of a review finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviewSeverity {
    Critical,
    High,
    Medium,
    Low,
    #[default]
    Info,
}

impl fmt::Display for ReviewSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewSeverity::Critical => write!(f, "critical"),
            ReviewSeverity::High => write!(f, "high"),
            ReviewSeverity::Medium => write!(f, "medium"),
            ReviewSeverity::Low => write!(f, "low"),
            ReviewSeverity::Info => write!(f, "info"),
        }
    }
}

/// Category of a review finding.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ReviewCategory {
    #[default]
    Correctness,
    Regression,
    Verification,
    PlanDeviation,
    Security,
    Architecture,
    Testing,
    Maintainability,
}

impl fmt::Display for ReviewCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewCategory::Correctness => write!(f, "correctness"),
            ReviewCategory::Regression => write!(f, "regression"),
            ReviewCategory::Verification => write!(f, "verification"),
            ReviewCategory::PlanDeviation => write!(f, "plan_deviation"),
            ReviewCategory::Security => write!(f, "security"),
            ReviewCategory::Architecture => write!(f, "architecture"),
            ReviewCategory::Testing => write!(f, "testing"),
            ReviewCategory::Maintainability => write!(f, "maintainability"),
        }
    }
}

/// One evidence-backed finding produced by the reviewer.
#[derive(Debug, Clone)]
pub struct ReviewFinding {
    pub severity: ReviewSeverity,
    pub category: ReviewCategory,
    pub title: String,
    /// The file the finding is anchored to, when available.
    pub file: Option<PathBuf>,
    /// The symbol the finding references, when available.
    pub symbol: Option<String>,
    /// Concise statement of the finding.
    pub statement: String,
    /// The tool observation or structured evidence that backs the finding.
    pub evidence: String,
    /// Concrete recommendation, when available.
    pub recommendation: String,
}

// =========================================================================
// Result
// =========================================================================

/// The verdict rendered by the reviewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviewVerdict {
    #[default]
    Pass,
    PassWithRisks,
    Fail,
}

impl fmt::Display for ReviewVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReviewVerdict::Pass => write!(f, "PASS"),
            ReviewVerdict::PassWithRisks => write!(f, "PASS_WITH_RISKS"),
            ReviewVerdict::Fail => write!(f, "FAIL"),
        }
    }
}

/// The structured result of one review session.
#[derive(Debug, Clone)]
pub struct ReviewResult {
    /// Human-readable executive summary.
    pub summary: String,
    /// Evidence-backed findings.
    pub findings: Vec<ReviewFinding>,
    /// Files the reviewer actually inspected via tools.
    pub reviewed_files: Vec<PathBuf>,
    /// Files the reviewer observed as changed via git diff.
    pub changed_files: Vec<PathBuf>,
    /// Planned files from the plan (when available).
    pub planned_changes: Vec<PathBuf>,
    /// Actual files changed according to git diff / CodingResult.
    pub actual_changes: Vec<PathBuf>,
    /// Changes covered by successful verification (from CodingResult).
    pub verified_changes: Vec<PathBuf>,
    /// Changes NOT covered by successful verification.
    pub unverified_changes: Vec<PathBuf>,
    /// Files changed that are outside the plan's affected files.
    pub plan_deviations: Vec<PathBuf>,
    /// Concrete security concerns with evidence.
    pub security_concerns: Vec<String>,
    /// Concrete regression risks.
    pub regression_risks: Vec<String>,
    /// Total real tool calls executed (all read-only).
    pub tool_calls: usize,
    /// Number of reasoning iterations.
    pub iterations: usize,
    /// Number of model (provider) calls.
    pub model_calls: usize,
    /// Why the session terminated.
    pub termination: ReviewTermination,
    /// Whether the final prose synthesis was actually produced.
    pub synthesis_complete: bool,
    /// Explicit limitations of this review pass.
    pub limitations: Vec<String>,
    /// Wall-clock duration.
    pub duration_ms: u64,
    /// Approximate output size in bytes.
    pub output_size: usize,
    /// Provider that executed the review.
    pub provider: String,
    /// Model used.
    pub model: String,
    /// The reviewer's final verdict.
    pub verdict: ReviewVerdict,
}

impl ReviewResult {
    /// A result for a session that never executed.
    pub fn failed(task: &str, termination: ReviewTermination, error: &str) -> Self {
        ReviewResult {
            summary: format!("Review for '{task}' did not complete: {error}"),
            findings: Vec::new(),
            reviewed_files: Vec::new(),
            changed_files: Vec::new(),
            planned_changes: Vec::new(),
            actual_changes: Vec::new(),
            verified_changes: Vec::new(),
            unverified_changes: Vec::new(),
            plan_deviations: Vec::new(),
            security_concerns: Vec::new(),
            regression_risks: Vec::new(),
            tool_calls: 0,
            iterations: 0,
            model_calls: 0,
            termination,
            synthesis_complete: false,
            limitations: vec![error.to_string()],
            duration_ms: 0,
            output_size: 0,
            provider: String::new(),
            model: String::new(),
            verdict: ReviewVerdict::Pass,
        }
    }

    /// Human-readable rendering used for the `review` ContextFragment.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "## Autonomous Review\n\nVerdict: {}\nTermination: {}\n",
            self.verdict, self.termination
        ));
        out.push_str(&format!(
            "Iterations: {} | tool calls: {} | model calls: {} | synthesis complete: {}\n",
            self.iterations, self.tool_calls, self.model_calls, self.synthesis_complete
        ));
        if !self.provider.is_empty() {
            out.push_str(&format!("Provider: {}\n", self.provider));
        }

        if !self.findings.is_empty() {
            out.push_str("\n## Findings\n");
            for finding in &self.findings {
                out.push_str(&format!(
                    "- [{}] {} — {}\n",
                    finding.severity, finding.category, finding.title
                ));
                if let Some(file) = &finding.file {
                    out.push_str(&format!("  file: {}\n", file.display()));
                }
                if let Some(symbol) = &finding.symbol {
                    out.push_str(&format!("  symbol: {}\n", symbol));
                }
                if !finding.evidence.is_empty() {
                    out.push_str(&format!("  evidence: {}\n", finding.evidence));
                }
                if !finding.recommendation.is_empty() {
                    out.push_str(&format!("  recommendation: {}\n", finding.recommendation));
                }
            }
        } else {
            out.push_str("\n(no structured findings)\n");
        }

        if !self.verified_changes.is_empty() || !self.unverified_changes.is_empty() {
            out.push_str("\n## Verification status\n");
            for f in &self.verified_changes {
                out.push_str(&format!("- {} [verified]\n", f.display()));
            }
            for f in &self.unverified_changes {
                out.push_str(&format!("- {} [UNVERIFIED]\n", f.display()));
            }
        }

        if !self.plan_deviations.is_empty() {
            out.push_str("\n## Plan deviations\n");
            for f in &self.plan_deviations {
                out.push_str(&format!("- {}\n", f.display()));
            }
        }

        if !self.security_concerns.is_empty() {
            out.push_str("\n## Security concerns\n");
            for s in &self.security_concerns {
                out.push_str(&format!("- {s}\n"));
            }
        }

        if !self.regression_risks.is_empty() {
            out.push_str("\n## Regression risks\n");
            for r in &self.regression_risks {
                out.push_str(&format!("- {r}\n"));
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
            "[review] provider={} model={} iterations={} tool_calls={} model_calls={} findings={} verdict={} termination={} synthesis={} duration={}ms output={}B",
            if self.provider.is_empty() { "-" } else { &self.provider },
            if self.model.is_empty() { "-" } else { &self.model },
            self.iterations,
            self.tool_calls,
            self.model_calls,
            self.findings.len(),
            self.verdict,
            self.termination,
            self.synthesis_complete,
            self.duration_ms,
            self.output_size,
        )
    }

    /// Record the provider/model that executed the review.
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
        assert_eq!(ReviewTermination::Completed.to_string(), "completed");
        assert_eq!(
            ReviewTermination::IterationLimit.to_string(),
            "iteration_limit"
        );
        assert_eq!(ReviewTermination::ToolLimit.to_string(), "tool_limit");
        assert_eq!(ReviewTermination::ModelLimit.to_string(), "model_limit");
        assert_eq!(ReviewTermination::Timeout.to_string(), "timeout");
        assert_eq!(ReviewTermination::Cancelled.to_string(), "cancelled");
        assert_eq!(ReviewTermination::Error.to_string(), "error");
    }

    #[test]
    fn test_verdict_display() {
        assert_eq!(ReviewVerdict::Pass.to_string(), "PASS");
        assert_eq!(ReviewVerdict::PassWithRisks.to_string(), "PASS_WITH_RISKS");
        assert_eq!(ReviewVerdict::Fail.to_string(), "FAIL");
    }

    #[test]
    fn test_failed_result_is_bounded_error_result() {
        let result = ReviewResult::failed(
            "review the work",
            ReviewTermination::Error,
            "provider offline",
        );
        assert_eq!(result.termination, ReviewTermination::Error);
        assert!(result.summary.contains("provider offline"));
        assert!(result.findings.is_empty());
        assert_eq!(result.tool_calls, 0);
        assert!(!result.synthesis_complete);
    }

    #[test]
    fn test_render_includes_findings_and_verification_status() {
        let mut result = ReviewResult::failed("review", ReviewTermination::Completed, "ok");
        result.findings.push(ReviewFinding {
            severity: ReviewSeverity::High,
            category: ReviewCategory::Verification,
            title: "unverified change".to_string(),
            file: Some(PathBuf::from("src/lib.rs")),
            symbol: Some("add".to_string()),
            statement: "no machine verification covered this change".to_string(),
            evidence: "exit_code: -1, success: false".to_string(),
            recommendation: "run cargo check".to_string(),
        });
        result.verified_changes = vec![PathBuf::from("src/main.rs")];
        result.unverified_changes = vec![PathBuf::from("src/lib.rs")];
        result.plan_deviations = vec![PathBuf::from("src/extra.rs")];
        result.security_concerns = vec!["no secrets found".to_string()];
        result.regression_risks = vec!["callers of add may break".to_string()];

        let rendered = result.render();
        assert!(rendered.contains("Autonomous Review"));
        assert!(rendered.contains("PASS"));
        assert!(rendered.contains("[high]"));
        assert!(rendered.contains("unverified change"));
        assert!(rendered.contains("src/lib.rs"));
        assert!(rendered.contains("[verified]"));
        assert!(rendered.contains("[UNVERIFIED]"));
        assert!(rendered.contains("Plan deviations"));
        assert!(rendered.contains("Security concerns"));
        assert!(rendered.contains("Regression risks"));
    }

    #[test]
    fn test_render_no_findings_is_honest() {
        let result = ReviewResult::failed("review", ReviewTermination::Completed, "ok");
        let rendered = result.render();
        assert!(rendered.contains("no structured findings"));
    }
}
