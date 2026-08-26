//! Root-cause hypothesis types.
//!
//! The output model is deliberately compact: OpenCode consumes it to decide
//! what to inspect next. Hypotheses are derived RUNTIME EVIDENCE — never
//! persisted, never written to the fact store, never mixed with
//! agent-recorded memory.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use serde::{Deserialize, Serialize};

/// Hard resource bounds for hypothesis generation. All loops are bounded by
/// these constants; nothing here executes commands or mutates state.
pub const MAX_DIAG_LOCATIONS: usize = 32;
pub const MAX_CANDIDATES: usize = 24;
pub const MAX_HYPOTHESES: usize = 5;
pub const MAX_EVIDENCE_PER_HYPOTHESIS: usize = 8;
/// Impact traversal budget per enriched candidate.
pub const IMPACT_DEPTH: usize = 1;
pub const IMPACT_MAX_RESULTS: usize = 10;
/// How many top-ranked candidates receive impact enrichment.
pub const MAX_IMPACT_ENRICHED: usize = 3;

/// Overall result status, so consumers can distinguish a real ranking from
/// an honest "there is not enough evidence" answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisStatus {
    /// At least one hypothesis carried non-trivial evidence.
    Hypotheses,
    /// Evidence existed but was too weak/ambiguous to rank confidently.
    WeakSignals,
    /// No usable evidence at all (e.g. success runs, denied runs).
    InsufficientEvidence,
}

/// One candidate symbol under suspicion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub symbol_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Span end when known (from the fact's SourceLocation.span).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
}

/// Evidence categories. Each kind maps to one explicit weight in the
/// ranking model; sources are recorded so duplicates cannot inflate scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// Diagnostic file+line lands inside the symbol's span.
    ExactDiagnosticLocation,
    /// Diagnostic file matches but the line is outside the span.
    DiagnosticFileMatch,
    /// A failing test exercises this symbol via `TestFact.tested`.
    FailingTestExercisesSymbol,
    /// The candidate IS the failing test's own function (the test itself
    /// may be wrong) — weak, kept separate so it cannot masquerade as
    /// root-cause evidence.
    CandidateIsFailingTest,
    /// A recent edit touched the same file as the candidate.
    RecentChangeMatch,
    /// This edit's advisory recommended a test that then failed — the
    /// precise causal hint from targeted selection ("changed recently",
    /// never "caused").
    RecentChangeRecommendedFailingTest,
    /// Verified AST relationship connects the candidate to failing evidence.
    VerifiedRelationship,
    /// Heuristic relationship connects them.
    HeuristicRelationship,
}

impl EvidenceKind {
    /// Explicit, transparent weights. Exact location > exercised-by-failing-
    /// test > verified relationship > recent change > heuristic/file-only.
    pub fn weight(self) -> f64 {
        match self {
            EvidenceKind::ExactDiagnosticLocation => 0.45,
            EvidenceKind::DiagnosticFileMatch => 0.12,
            EvidenceKind::FailingTestExercisesSymbol => 0.30,
            EvidenceKind::CandidateIsFailingTest => 0.08,
            EvidenceKind::RecentChangeMatch => 0.12,
            EvidenceKind::RecentChangeRecommendedFailingTest => 0.18,
            EvidenceKind::VerifiedRelationship => 0.20,
            EvidenceKind::HeuristicRelationship => 0.08,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceKind::ExactDiagnosticLocation => "exact_diagnostic_location",
            EvidenceKind::DiagnosticFileMatch => "diagnostic_file_match",
            EvidenceKind::FailingTestExercisesSymbol => "failing_test_exercises_symbol",
            EvidenceKind::CandidateIsFailingTest => "candidate_is_failing_test",
            EvidenceKind::RecentChangeMatch => "recent_change_match",
            EvidenceKind::RecentChangeRecommendedFailingTest => {
                "recent_change_recommended_failing_test"
            }
            EvidenceKind::VerifiedRelationship => "verified_relationship",
            EvidenceKind::HeuristicRelationship => "heuristic_relationship",
        }
    }
}

/// One piece of supporting evidence attached to a hypothesis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub kind: EvidenceKind,
    /// What the evidence came from: diagnostic index, test name/symbol id,
    /// edited path, or relationship id.
    pub source: String,
    /// Human-readable explanation surfaced to the agent.
    pub detail: String,
}

/// Compact structural context from impact-engine for one candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpactEdgeSummary {
    pub kind: String,
    pub direction: String,
    pub other_symbol: String,
    pub provenance: String,
    pub depth: usize,
}

/// A ranked root-cause hypothesis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hypothesis {
    pub candidate: Candidate,
    /// Bounded [0,1]; deterministic weighted sum after penalties.
    pub score: f64,
    /// `strong` | `moderate` | `weak` | `insufficient`.
    pub confidence: String,
    pub evidence: Vec<Evidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub impact_context: Vec<ImpactEdgeSummary>,
    /// Recent edits correlated with this candidate ("changed recently" —
    /// never claimed as "caused").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_recent_changes: Vec<serde_json::Value>,
    /// Failing tests that exercise this candidate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_tests: Vec<String>,
    /// Set when several symbols share the candidate's name.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ambiguous_name: bool,
}

/// Top-level structured result embedded additively into sandbox_test /
/// sandbox_build verification payloads as `root_cause`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RootCauseAnalysis {
    pub status: AnalysisStatus,
    pub failure_classification: String,
    pub hypotheses: Vec<Hypothesis>,
    /// Counts per evidence kind across all hypotheses — quick shape summary.
    pub evidence_summary: Vec<(String, usize)>,
    pub freshness: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
}
