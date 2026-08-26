//! Phase-9 hardening suite: pins the trustworthiness invariants of the
//! root-cause hypothesis engine.
//!
//! Covered invariants (each maps to a Phase-9 audit dimension):
//! - correlated evidence must not inflate scores (dedup by (kind, source));
//! - a diagnostic without line precision must never earn exact-location
//!   weight;
//! - ranking calibration ordering (exact > file-match, tested-linkage >
//!   recency, verified > heuristic, fresh > unknown > stale);
//! - ambiguity is preserved, penalized and surfaced, never silently
//!   resolved;
//! - impact enrichment cannot outweigh direct diagnostic evidence and is
//!   truncated honestly (displayed evidence == scored evidence);
//! - the A→B→C causal chain exposes only depth-supported leads;
//! - hard resource bounds hold under large synthetic failures;
//! - equivalent evidence quality ranks comparably across languages;
//! - output is byte-deterministic.

use codebro_mcp_server::debugging::candidates::RecentEditInput;
use codebro_mcp_server::debugging::ranking;
use codebro_mcp_server::debugging::types::{
    AnalysisStatus, EvidenceKind, MAX_EVIDENCE_PER_HYPOTHESIS, MAX_HYPOTHESES,
};
use codebro_mcp_server::debugging::{analyze_root_cause, RootCauseInput};
use codebro_mcp_server::engineering_facts::{
    metadata::FactMetadata, FactId, FactsBuilder, ModuleFact, ModuleId, Position, RelationshipFact,
    RelationshipId, RelationshipKind, SourceLocation, Span, SymbolFact, SymbolId, SymbolKind,
    TestFact, TestId, WorkspaceFact, WorkspaceId,
};
use codebro_mcp_server::fact_store::FactStore;
use codebro_mcp_server::sandbox::{ExecutionResult, ParsedDiagnostic, VerificationResult};

// ── fixture helpers ──────────────────────────────────────────────────────

fn verification(classification: &str, diags: Vec<ParsedDiagnostic>) -> VerificationResult {
    let execution = ExecutionResult::from_local(
        "verify",
        "/tmp/nowhere",
        "",
        "",
        1,
        10,
        false,
        false,
        std::collections::HashMap::new(),
    );
    let mut v = VerificationResult::from_execution(execution);
    v.verified = false;
    v.classification = Some(classification.to_string());
    v.diagnostics = diags;
    v
}

fn diag(file: Option<&str>, line: Option<u32>, test: Option<&str>) -> ParsedDiagnostic {
    ParsedDiagnostic {
        severity: if test.is_some() { "failure" } else { "error" }.to_string(),
        code: None,
        message: String::new(),
        file: file.map(|s| s.to_string()),
        line,
        column: None,
        test: test.map(|s| s.to_string()),
    }
}

fn run(
    classification: &str,
    diags: Vec<ParsedDiagnostic>,
    edits: Vec<RecentEditInput>,
    freshness: &str,
    store: &FactStore,
) -> codebro_mcp_server::debugging::types::RootCauseAnalysis {
    let v = verification(classification, diags);
    analyze_root_cause(RootCauseInput {
        verification: &v,
        store,
        workspace_root: std::path::Path::new("/tmp/nowhere"),
        freshness,
        recent_edits: &edits,
    })
}

fn edit(path: &str, recommended: &[&str]) -> RecentEditInput {
    RecentEditInput {
        path: path.to_string(),
        seconds_ago: 5,
        recommended_tests: recommended.iter().map(|s| s.to_string()).collect(),
    }
}

/// Incremental fact-model fixture builder.
struct Fx {
    b: FactsBuilder,
    modules: std::collections::BTreeSet<String>,
}

impl Fx {
    fn new(name: &str) -> Self {
        let mut b = FactsBuilder::new();
        b.add_workspace(WorkspaceFact::new(
            WorkspaceId::new(format!("ws::{name}")),
            name,
        ));
        Fx {
            b,
            modules: Default::default(),
        }
    }

    /// Register a module shell for `file` (call before/after adding symbols;
    /// modules are flushed at build time so ordering does not matter).
    fn track_file(&mut self, file: &str) {
        self.modules.insert(file.to_string());
    }

    /// Add a function-like symbol spanning [start, end]; returns its id.
    fn func(&mut self, name: &str, file: &str, start: u32, end: u32) -> String {
        self.track_file(file);
        let id = format!("sym::{file}::{name}_function@{start}");
        let mut sf = SymbolFact::new(SymbolId::new(id.clone()), name, SymbolKind::Function);
        sf.location = SourceLocation::new()
            .with_file(file)
            .with_span(Span::new(Position::new(start, 0), Position::new(end, 0)));
        sf.location.line = Some(start);
        self.b.add_symbol(sf);
        id
    }

    /// Add a test fact. `self_id` wires the test's own function symbol
    /// (`tf.target`); `tested` lists exercised symbol ids.
    fn test(&mut self, name: &str, self_id: Option<&str>, tested: &[&str]) {
        let mut tf = TestFact::new(TestId::new(format!("test::f::{name}")), name.to_string());
        if let Some(sid) = self_id {
            tf.target = Some(FactId::Symbol(SymbolId::new(sid.to_string())));
        }
        for t in tested {
            tf.tested.push(SymbolId::new(t.to_string()));
        }
        self.b.add_test(tf);
    }

    /// Add a Calls relationship with optional provenance override
    /// (`None` defaults to verified in the impact engine).
    fn calls(&mut self, src: &str, dst: &str, provenance: Option<&str>) {
        let mut rel = RelationshipFact::new(
            RelationshipId::new(format!("rel::{src}->{dst}")),
            RelationshipKind::Calls,
            FactId::Symbol(SymbolId::new(src.to_string())),
            FactId::Symbol(SymbolId::new(dst.to_string())),
        );
        if let Some(p) = provenance {
            rel.metadata = FactMetadata::builder().attr("provenance", p).build();
        }
        self.b.add_relationship(rel);
    }

    fn build(mut self) -> FactStore {
        for f in &self.modules {
            let mut m = ModuleFact::new(ModuleId::new(format!("mod::{f}")), f.replace('/', "::"));
            m.path = Some(f.clone());
            self.b.add_module(m);
        }
        FactStore::build(self.b.build())
    }
}

fn score_of<'a>(
    r: &'a codebro_mcp_server::debugging::types::RootCauseAnalysis,
    name: &str,
) -> &'a codebro_mcp_server::debugging::types::Hypothesis {
    r.hypotheses
        .iter()
        .find(|h| h.candidate.name == name)
        .unwrap_or_else(|| panic!("no hypothesis for {name}: {:?}", r.hypotheses))
}

const EPS: f64 = 1e-9;

// ── 1. evidence independence / correlated-evidence cases ────────────────

/// CASE A: only an exact diagnostic location exists → pinned at its single
/// weight, nothing else.
#[test]
fn case_a_exact_location_alone_is_pinned_to_single_weight() {
    let s = {
        let mut f = Fx::new("a");
        f.func("add", "src/lib.rs", 1, 3);
        f.build()
    };
    let r = run(
        "compile_error",
        vec![diag(Some("src/lib.rs"), Some(2), None)],
        vec![],
        "fresh",
        &s,
    );
    let h = score_of(&r, "add");
    assert!((h.score - 0.45).abs() < EPS, "got {}", h.score);
    assert_eq!(h.confidence, "moderate");
    assert_eq!(h.evidence.len(), 1);
}

/// CASE B: several diagnostics about the same file collapse to the single
/// strongest location channel per candidate — in-span → Exact (file-match
/// subsumed), out-of-span/file-only → FileMatch. Repeated representations
/// of one event must not stack into inflated confidence.
#[test]
fn case_b_correlated_file_channels_deduplicate() {
    let s = {
        let mut f = Fx::new("b");
        f.func("add", "src/lib.rs", 1, 3);
        f.func("unrelated_far", "src/lib.rs", 40, 45);
        f.build()
    };
    let r = run(
        "compile_error",
        vec![
            diag(Some("src/lib.rs"), Some(2), None),
            diag(Some("src/lib.rs"), Some(50), None),
            diag(Some("src/lib.rs"), None, None),
        ],
        vec![],
        "fresh",
        &s,
    );
    let add = score_of(&r, "add");
    // add: one Exact only — the out-of-span and file-only diagnostics are
    // weaker representations of the same file signal and are subsumed.
    assert_eq!(add.evidence.len(), 1, "{:?}", add.evidence);
    assert!((add.score - 0.45).abs() < EPS, "got {}", add.score);
    assert_eq!(add.confidence, "moderate");
    // far: no diagnostic lands in its span → pure file-match channel.
    let far = score_of(&r, "unrelated_far");
    assert_eq!(far.evidence.len(), 1);
    assert!(far
        .evidence
        .iter()
        .all(|e| e.kind == EvidenceKind::DiagnosticFileMatch));
    assert!((far.score - 0.12).abs() < EPS);
}

/// CASE C: the same underlying event represented twice through the same
/// channel (identical duplicate diagnostics) must not inflate.
#[test]
fn case_c_same_event_through_duplicate_diagnostics_does_not_inflate() {
    let s = {
        let mut f = Fx::new("c");
        let add = f.func("add", "src/lib.rs", 1, 3);
        f.test("adds", None, &[&add]);
        f.build()
    };
    let one = run(
        "test_failure",
        vec![diag(Some("src/lib.rs"), Some(2), Some("adds"))],
        vec![],
        "fresh",
        &s,
    );
    let two = run(
        "test_failure",
        vec![
            diag(Some("src/lib.rs"), Some(2), Some("adds")),
            diag(Some("src/lib.rs"), Some(2), Some("adds")),
        ],
        vec![],
        "fresh",
        &s,
    );
    let h1 = score_of(&one, "add");
    let h2 = score_of(&two, "add");
    assert!(
        (h1.score - h2.score).abs() < EPS,
        "{} vs {}",
        h1.score,
        h2.score
    );
    let exact_count = h2
        .evidence
        .iter()
        .filter(|e| e.kind == EvidenceKind::ExactDiagnosticLocation)
        .count();
    let tested_count = h2
        .evidence
        .iter()
        .filter(|e| e.kind == EvidenceKind::FailingTestExercisesSymbol)
        .count();
    assert_eq!(exact_count, 1);
    assert_eq!(tested_count, 1);
}

/// CASE D: one recent-edit event yields exactly ONE recency item (either
/// the precise recommendation bonus or the generic match, never both), and
/// the full realistic chain (tested linkage + edit advisory + verified AST
/// edge) lands strong while weaker variants do not.
#[test]
fn case_d_recent_event_single_channel_and_full_chain_calibration() {
    // Minimal chain: failing test exercises symbol; the edit advisory had
    // recommended precisely that failing test.
    let s = {
        let mut f = Fx::new("d");
        let add = f.func("add", "src/lib.rs", 1, 3);
        f.test("adds", None, &[&add]);
        f.build()
    };
    let edits = vec![edit("src/lib.rs", &["adds"])];
    let r = run(
        "test_failure",
        vec![diag(None, None, Some("adds"))],
        edits,
        "fresh",
        &s,
    );
    let h = score_of(&r, "add");
    let recent_items = h
        .evidence
        .iter()
        .filter(|e| {
            e.kind == EvidenceKind::RecentChangeMatch
                || e.kind == EvidenceKind::RecentChangeRecommendedFailingTest
        })
        .count();
    assert_eq!(recent_items, 1, "one edit event → one recency item");
    assert!(h
        .evidence
        .iter()
        .any(|e| e.kind == EvidenceKind::RecentChangeRecommendedFailingTest));
    // tested linkage (0.30) + precise recency (0.18) stays below strong.
    assert!((h.score - 0.48).abs() < EPS, "got {}", h.score);
    assert_eq!(h.confidence, "moderate");

    // Full chain adds a VERIFIED AST edge between the test function and the
    // symbol — a genuinely additional structural observation → strong.
    let s2 = {
        let mut f = Fx::new("d2");
        let add = f.func("add", "src/lib.rs", 1, 3);
        let adds_fn = f.func("adds", "src/lib.rs", 5, 8);
        f.calls(&adds_fn, &add, Some("verified"));
        f.test("adds", None, &[&add]);
        f.build()
    };
    let r2 = run(
        "test_failure",
        vec![diag(None, None, Some("adds"))],
        vec![edit("src/lib.rs", &["adds"])],
        "fresh",
        &s2,
    );
    let h2 = score_of(&r2, "add");
    assert!(
        h2.evidence
            .iter()
            .any(|e| e.kind == EvidenceKind::VerifiedRelationship),
        "{:?}",
        h2.evidence
    );
    assert!((h2.score - 0.68).abs() < EPS, "got {}", h2.score);
    assert_eq!(h2.confidence, "strong");
}

/// Regression pin: a diagnostic carrying a file but NO line (the common
/// pytest short-summary shape) grants only DiagnosticFileMatch weight to
/// every symbol in that file — never ExactDiagnosticLocation.
#[test]
fn file_only_diagnostic_never_earns_exact_location_weight() {
    let s = {
        let mut f = Fx::new("noline");
        f.func("helper_a", "tests/test_x.py", 1, 6);
        f.func("helper_b", "tests/test_x.py", 10, 15);
        f.build()
    };
    let r = run(
        "test_failure",
        vec![diag(Some("tests/test_x.py"), None, Some("test_y"))],
        vec![],
        "fresh",
        &s,
    );
    for name in ["helper_a", "helper_b"] {
        let h = score_of(&r, name);
        assert_eq!(h.evidence.len(), 1, "{name}: {:?}", h.evidence);
        assert_eq!(h.evidence[0].kind, EvidenceKind::DiagnosticFileMatch);
        assert!((h.score - 0.12).abs() < EPS, "{name}: {}", h.score);
        assert_eq!(h.confidence, "weak");
    }
}

/// Regression pin: a blank runner test name must not suffix-match every
/// test fact in the store (which would fabricate failing-test evidence for
/// unrelated symbols and blow up registration cost).
#[test]
fn blank_runner_test_names_are_ignored() {
    let s = {
        let mut f = Fx::new("blank");
        let a = f.func("alpha", "src/a.rs", 1, 4);
        let b = f.func("beta", "src/b.rs", 1, 4);
        f.test("test_alpha", None, &[&a]);
        f.test("test_beta", None, &[&b]);
        f.build()
    };
    let r = run(
        "test_failure",
        vec![diag(None, None, Some(""))],
        vec![],
        "fresh",
        &s,
    );
    assert!(r.hypotheses.is_empty(), "{:?}", r.hypotheses);
    assert_eq!(r.status, AnalysisStatus::InsufficientEvidence);
}

// ── 2. ranking calibration matrix ────────────────────────────────────────

/// Matrix ordering principles, each measured against the same store shape:
/// exact > generic-file, tested-linkage > recency, fresh > unknown > stale,
/// ambiguity penalizes, no evidence → insufficient.
#[test]
fn calibration_matrix_orders_sensibly() {
    let mk = || {
        let mut f = Fx::new("matrix");
        let add = f.func("add", "src/lib.rs", 1, 3);
        f.test("adds", None, &[&add]);
        f.build()
    };

    // A exact (0.45) vs F generic file match (0.12).
    let s = mk();
    let exact = run(
        "compile_error",
        vec![diag(Some("src/lib.rs"), Some(2), None)],
        vec![],
        "fresh",
        &s,
    );
    let file_only = run(
        "unknown_failure",
        vec![diag(Some("src/lib.rs"), None, None)],
        vec![],
        "fresh",
        &s,
    );
    assert!(score_of(&exact, "add").score > score_of(&file_only, "add").score);

    // B tested-linkage (0.30) vs E recency-only (0.12).
    let tested = run(
        "test_failure",
        vec![diag(None, None, Some("adds"))],
        vec![],
        "fresh",
        &s,
    );
    let recency = run(
        "unknown_failure",
        vec![diag(None, None, None)],
        vec![edit("src/lib.rs", &[])],
        "fresh",
        &s,
    );
    assert!(score_of(&tested, "add").score > score_of(&recency, "add").score);

    // H/I freshness: fresh > unknown > stale on identical inputs.
    let fresh = run(
        "compile_error",
        vec![diag(Some("src/lib.rs"), Some(2), None)],
        vec![],
        "fresh",
        &s,
    );
    let unknown = run(
        "compile_error",
        vec![diag(Some("src/lib.rs"), Some(2), None)],
        vec![],
        "unknown",
        &s,
    );
    let stale = run(
        "compile_error",
        vec![diag(Some("src/lib.rs"), Some(2), None)],
        vec![],
        "stale",
        &s,
    );
    let (f, u, st) = (
        score_of(&fresh, "add").score,
        score_of(&unknown, "add").score,
        score_of(&stale, "add").score,
    );
    assert!(f > u && u > st, "fresh={f} unknown={u} stale={st}");
    assert!((f - 0.45).abs() < EPS);
    assert!((u - 0.45 * 0.95).abs() < EPS);
    assert!((st - 0.45 * 0.85).abs() < EPS);
    assert!(stale.limitations.iter().any(|l| l.contains("stale")));
    assert!(unknown
        .limitations
        .iter()
        .any(|l| l.contains("could not be determined")));

    // J: no evidence at all → insufficient, no hypotheses.
    let empty = run("success", vec![], vec![], "fresh", &s);
    assert_eq!(empty.status, AnalysisStatus::InsufficientEvidence);
    assert!(empty.hypotheses.is_empty());

    // Combined direct evidence beats combined supporting evidence:
    // exact+tested (0.75) > tested+precise-recency (0.48).
    let direct = run(
        "test_failure",
        vec![diag(Some("src/lib.rs"), Some(2), Some("adds"))],
        vec![],
        "fresh",
        &s,
    );
    let supporting = run(
        "test_failure",
        vec![diag(None, None, Some("adds"))],
        vec![edit("src/lib.rs", &["adds"])],
        "fresh",
        &s,
    );
    assert!(score_of(&direct, "add").score > score_of(&supporting, "add").score);
}

/// Verified relationship evidence must outrank the identical structure with
/// heuristic provenance, through the REAL enrichment path.
#[test]
fn verified_relationship_outranks_heuristic_via_enrichment() {
    let build = |prov: &str| {
        let mut f = Fx::new(prov);
        let calc = f.func("calc", "src/math.rs", 1, 8);
        let check_fn = f.func("check", "src/math.rs", 10, 14);
        f.calls(&check_fn, &calc, Some(prov));
        f.test("check", None, &[&calc]);
        f.build()
    };
    let verified = run(
        "test_failure",
        vec![diag(None, None, Some("check"))],
        vec![],
        "fresh",
        &build("verified"),
    );
    let heuristic = run(
        "test_failure",
        vec![diag(None, None, Some("check"))],
        vec![],
        "fresh",
        &build("heuristic"),
    );
    let hv = score_of(&verified, "calc");
    let hh = score_of(&heuristic, "calc");
    assert!(hv
        .evidence
        .iter()
        .any(|e| e.kind == EvidenceKind::VerifiedRelationship));
    assert!(hh
        .evidence
        .iter()
        .any(|e| e.kind == EvidenceKind::HeuristicRelationship));
    assert!((hv.score - 0.50).abs() < EPS, "verified {}", hv.score);
    assert!((hh.score - 0.38).abs() < EPS, "heuristic {}", hh.score);
    assert!(hv.score > hh.score);
    // Neither combination may reach strong on relationship corroboration
    // alone: the underlying call edge partially overlaps the tested link.
    assert_eq!(hv.confidence, "moderate");
}

// ── 3. ambiguity safety ──────────────────────────────────────────────────

#[test]
fn two_equally_plausible_candidates_are_both_preserved_and_flagged() {
    let s = {
        let mut f = Fx::new("amb2");
        let ha = f.func("handler", "src/a.rs", 1, 6);
        let hb = f.func("handler", "src/b.rs", 1, 6);
        f.test("it_fails", None, &[&ha, &hb]);
        f.build()
    };
    let r = run(
        "test_failure",
        vec![diag(None, None, Some("it_fails"))],
        vec![],
        "fresh",
        &s,
    );
    let hs: Vec<_> = r
        .hypotheses
        .iter()
        .filter(|h| h.candidate.name == "handler")
        .collect();
    assert_eq!(hs.len(), 2, "both candidates must survive");
    assert!(hs.iter().all(|h| h.ambiguous_name));
    assert!(hs.iter().all(|h| (h.score - (0.30 - 0.15)).abs() < EPS));
    // Deterministic tiebreak: symbol id ascending.
    let mut ids: Vec<&str> = hs.iter().map(|h| h.candidate.symbol_id.as_str()).collect();
    ids.sort_unstable();
    let ordered: Vec<&str> = r
        .hypotheses
        .iter()
        .filter(|h| h.candidate.name == "handler")
        .map(|h| h.candidate.symbol_id.as_str())
        .collect();
    assert_eq!(ids, ordered);
    assert!(r.limitations.iter().any(|l| l.contains("handler")));
}

#[test]
fn three_way_ambiguity_is_preserved_and_penalized_uniformly() {
    let s = {
        let mut f = Fx::new("amb3");
        let h1 = f.func("resolver", "src/x.rs", 1, 5);
        let h2 = f.func("resolver", "src/y.rs", 1, 5);
        let h3 = f.func("resolver", "src/z.rs", 1, 5);
        f.test("t3", None, &[&h1, &h2, &h3]);
        f.build()
    };
    let r = run(
        "test_failure",
        vec![diag(None, None, Some("t3"))],
        vec![],
        "fresh",
        &s,
    );
    let hs: Vec<_> = r
        .hypotheses
        .iter()
        .filter(|h| h.candidate.name == "resolver")
        .collect();
    assert_eq!(hs.len(), 3);
    assert!(hs.iter().all(|h| h.ambiguous_name));
    assert!(hs
        .iter()
        .all(|h| (h.score - (0.30 - 0.15)).abs() < EPS && h.confidence == "weak"));
    assert!(r.limitations.iter().any(|l| l.contains("resolver")));
}

#[test]
fn ambiguous_candidate_with_exact_location_stays_top_but_flagged() {
    // Both stores give their target symbol identical direct evidence
    // (exact location + failing-test linkage); the ambiguous store adds a
    // SECOND symbol with the same name that is also exercised by the test,
    // so the name genuinely collides inside the candidate set.
    let mk_amb = || {
        let mut f = Fx::new("ambe");
        let ha = f.func("handler", "src/a.rs", 1, 10);
        let hb = f.func("handler", "src/b.rs", 1, 10);
        f.test("t", None, &[&ha, &hb]);
        f.build()
    };
    let mk_uniq = || {
        let mut f = Fx::new("uniq");
        let ha = f.func("handler", "src/a.rs", 1, 10);
        let other = f.func("other_fn", "src/c.rs", 1, 10);
        f.test("t", None, &[&ha, &other]);
        f.build()
    };
    let diags = vec![diag(Some("src/a.rs"), Some(5), Some("t"))];
    let amb = run("test_failure", diags.clone(), vec![], "fresh", &mk_amb());
    let uniq = run("test_failure", diags, vec![], "fresh", &mk_uniq());

    let ha = score_of(&amb, "handler");
    assert_eq!(amb.hypotheses[0].candidate.name, "handler");
    assert_eq!(
        amb.hypotheses[0].candidate.file.as_deref(),
        Some("src/a.rs"),
        "exact location disambiguates ranking"
    );
    assert!(ha.ambiguous_name);
    // Penalized by exactly −0.15 relative to the identical unambiguous setup.
    let hu = score_of(&uniq, "handler");
    assert!(
        (hu.score - ha.score - 0.15).abs() < EPS,
        "{} vs {}",
        hu.score,
        ha.score
    );
    assert!(amb.limitations.iter().any(|l| l.contains("handler")));

    // The same-name decoy survives as a separate, weaker hypothesis — it is
    // never silently merged away.
    let hb = amb
        .hypotheses
        .iter()
        .find(|h| h.candidate.file.as_deref() == Some("src/b.rs"))
        .expect("same-name decoy preserved");
    assert!(hb.ambiguous_name);
    assert!(hb.score < ha.score);
}

#[test]
fn ambiguous_recency_only_is_not_promoted() {
    let s = {
        let mut f = Fx::new("ambr");
        f.func("worker", "src/f.rs", 1, 5);
        f.func("worker", "src/g.rs", 1, 5);
        f.build()
    };
    let r = run(
        "unknown_failure",
        vec![diag(None, None, None)],
        vec![edit("src/f.rs", &[])],
        "fresh",
        &s,
    );
    assert!(r.hypotheses.iter().all(|h| h.confidence != "strong"));
    assert!(r.hypotheses.iter().all(|h| h.confidence != "moderate"));
    assert_eq!(r.status, AnalysisStatus::WeakSignals);
}

// ── 4. impact enrichment ─────────────────────────────────────────────────

/// A candidate with direct exact-location evidence must outrank a candidate
/// with a much larger impact graph and only weak recency evidence.
#[test]
fn exact_evidence_beats_larger_impact_graph() {
    let s = {
        let mut f = Fx::new("graph");
        f.func("victim", "src/a.rs", 1, 10);
        let hub = f.func("hub", "src/b.rs", 1, 10);
        for i in 0..12 {
            let u = f.func(&format!("util{i}"), "src/b.rs", 100 + i * 10, 105 + i * 10);
            f.calls(&hub, &u, Some("verified"));
        }
        f.build()
    };
    let r = run(
        "compile_error",
        vec![diag(Some("src/a.rs"), Some(5), None)],
        vec![edit("src/b.rs", &[])],
        "fresh",
        &s,
    );
    let victim = score_of(&r, "victim");
    let hub = score_of(&r, "hub");
    assert!(
        victim.score > hub.score,
        "victim {} vs hub {}",
        victim.score,
        hub.score
    );
    assert!(victim
        .evidence
        .iter()
        .any(|e| e.kind == EvidenceKind::ExactDiagnosticLocation));
    // Hub's graph is attached as CONTEXT, not as relationship evidence: no
    // edge connects hub to failing evidence here.
    assert!(!hub.evidence.iter().any(|e| matches!(
        e.kind,
        EvidenceKind::VerifiedRelationship | EvidenceKind::HeuristicRelationship
    )));
    assert!(!hub.impact_context.is_empty());
    assert!(hub.impact_context.len() <= 10, "impact summaries capped");
    assert!(victim.impact_context.is_empty());
}

/// Displayed evidence and scored evidence must agree after enrichment, and
/// the per-hypothesis evidence cap must hold.
#[test]
fn enrichment_truncates_and_scores_displayed_evidence_only() {
    let s = {
        let mut f = Fx::new("cap");
        let core = f.func("core", "src/c.rs", 1, 20);
        f.test("t1", None, &[&core]);
        f.test("t2", None, &[&core]);
        f.test("t3", None, &[&core]);
        f.test("t4", None, &[&core]);
        f.test("t5", None, &[&core]);
        f.build()
    };
    // Five independent failing tests exercise `core` (distinct sources →
    // distinct evidence) plus exact location and recency channels.
    let r = run(
        "test_failure",
        vec![
            diag(Some("src/c.rs"), Some(3), Some("t1")),
            diag(None, None, Some("t2")),
            diag(None, None, Some("t3")),
            diag(None, None, Some("t4")),
            diag(None, None, Some("t5")),
        ],
        vec![edit("src/c.rs", &["t1"])],
        "fresh",
        &s,
    );
    let h = score_of(&r, "core");
    assert!(
        h.evidence.len() <= MAX_EVIDENCE_PER_HYPOTHESIS,
        "{}",
        h.evidence.len()
    );
    // Recompute the score from the DISPLAYED evidence — they must match.
    let recomputed = ranking::base_score(&h.evidence);
    let (expected, _) = ranking::adjusted(recomputed, h.ambiguous_name, "fresh");
    assert!(
        (expected - h.score).abs() < 1e-9,
        "displayed {} vs scored {}",
        recomputed,
        h.score
    );
    // Score cap still respected.
    assert!(h.score <= 0.98 + EPS);
}

// ── 5. candidate completeness: A → B → C chain ───────────────────────────

/// The diagnostic points at C, B (likely cause) merely calls C, A was
/// recently changed. Candidate generation anchors on observed evidence
/// only: B is neither in the diagnostic span nor test-linked nor edited,
/// so it must NOT appear as a fabricated hypothesis — the engine returns
/// limited leads instead of an invented causal chain.
#[test]
fn chain_depth_limitation_is_explicit_not_misleading() {
    let s = {
        let mut f = Fx::new("chain");
        f.func("c_fn", "src/chain.rs", 20, 25);
        f.func("b_fn", "src/chain.rs", 10, 15);
        f.func("a_fn", "src/other.rs", 1, 5);
        f.build()
    };
    let r = run(
        "compile_error",
        vec![diag(Some("src/chain.rs"), Some(22), None)],
        vec![edit("src/other.rs", &[])],
        "fresh",
        &s,
    );
    let c = score_of(&r, "c_fn");
    assert_eq!(c.confidence, "moderate"); // exact location, nothing more
                                          // B (the unobserved intermediate) is absent entirely — never invented.
    assert!(r.hypotheses.iter().all(|h| h.candidate.name != "b_fn"));
    let a = score_of(&r, "a_fn");
    assert!(a
        .evidence
        .iter()
        .all(|e| e.kind == EvidenceKind::RecentChangeMatch));
    assert!(a.score < c.score);
    // No relationship evidence anywhere: no failing-test links exist for
    // enrichment to key on, so structural edges stay context-only.
    assert!(r
        .hypotheses
        .iter()
        .all(|h| h.evidence.iter().all(|e| !matches!(
            e.kind,
            EvidenceKind::VerifiedRelationship | EvidenceKind::HeuristicRelationship
        ))));
    // Nothing reaches strong → the engine says so explicitly.
    assert!(r
        .limitations
        .iter()
        .any(|l| l.contains("below strong threshold")));
    assert_eq!(r.status, AnalysisStatus::WeakSignals);
}

// ── 6. bounds / performance ──────────────────────────────────────────────

/// Large synthetic failure: many diagnostics (more than processed), heavy
/// name reuse, many edits. Everything must stay bounded and fast.
#[test]
fn large_synthetic_failure_stays_bounded() {
    let mut f = Fx::new("big");
    for file_i in 0..40 {
        let path = format!("src/mod{file_i}.rs");
        for sym_i in 0..8 {
            let name = if sym_i % 4 == 0 {
                "util".to_string() // heavy name reuse → ambiguity pressure
            } else {
                format!("f{file_i}_{sym_i}")
            };
            f.func(&name, &path, 1 + sym_i * 10, 5 + sym_i * 10);
        }
    }
    let s = f.build();

    let mut diags = Vec::new();
    for i in 0..40 {
        diags.push(diag(
            Some(&format!("src/mod{}.rs", i % 40)),
            Some(3 + (i % 8) * 10),
            None,
        ));
    }
    let edits: Vec<RecentEditInput> = (0..12)
        .map(|i| edit(&format!("src/mod{i}.rs"), &[]))
        .collect();

    let started = std::time::Instant::now();
    let r = run("compile_error", diags, edits, "stale", &s);
    let elapsed = started.elapsed();

    assert!(elapsed.as_millis() < 5_000, "analysis took {elapsed:?}");
    assert!(r.hypotheses.len() <= MAX_HYPOTHESES);
    assert!(r.limitations.len() <= 16);
    for h in &r.hypotheses {
        assert!(h.evidence.len() <= MAX_EVIDENCE_PER_HYPOTHESIS + 2);
        assert!(h.impact_context.len() <= 10);
        assert!((0.0..=0.98).contains(&h.score));
    }
    // Freshness surfaced honestly.
    assert_eq!(r.freshness, "stale");
    assert!(r.limitations.iter().any(|l| l.contains("stale")));
}

// ── 7. cross-language equivalence ────────────────────────────────────────

/// Equivalent "failing test exercises symbol" evidence produces identical
/// scores/tiers regardless of language naming conventions; language-specific
/// diagnostic shapes map onto the documented channels.
#[test]
fn cross_language_equivalent_evidence_ranks_comparably() {
    // Rust: `test adds ... FAILED` → failure diag with test name.
    let rust = {
        let mut f = Fx::new("lang_rust");
        let add = f.func("add", "src/lib.rs", 1, 3);
        f.test("adds", None, &[&add]);
        (f.build(), "add".to_string())
    };
    // Go: `--- FAIL: TestAdd` → bare failure diag with test name.
    let go = {
        let mut f = Fx::new("lang_go");
        let add = f.func("Add", "math.go", 1, 3);
        f.test("TestAdd", None, &[&add]);
        (f.build(), "Add".to_string())
    };
    // Python: pytest summary carries file + node tail (NO line number).
    let py = {
        let mut f = Fx::new("lang_py");
        let add = f.func("add", "src/ops.py", 1, 3);
        f.func("py_helper", "tests/test_ops.py", 1, 4);
        f.test("test_add", None, &[&add]);
        (f.build(), "add".to_string())
    };

    let cases = [
        (
            rust.0,
            diag(None, None, Some("adds")),
            rust.1,
            EvidenceKind::FailingTestExercisesSymbol,
        ),
        (
            go.0,
            diag(None, None, Some("TestAdd")),
            go.1,
            EvidenceKind::FailingTestExercisesSymbol,
        ),
        (
            py.0,
            diag(Some("tests/test_ops.py"), None, Some("test_add")),
            py.1,
            EvidenceKind::FailingTestExercisesSymbol,
        ),
    ];
    for (store, d, sym, expected_kind) in &cases {
        let r = run("test_failure", vec![d.clone()], vec![], "fresh", store);
        let h = score_of(&r, sym);
        assert!(
            h.evidence.iter().any(|e| e.kind == *expected_kind),
            "{sym}: {:?}",
            h.evidence
        );
        assert!((h.score - 0.30).abs() < EPS, "{sym}: {}", h.score);
        assert_eq!(h.confidence, "moderate", "{sym}");
    }

    // Python-specific: the same-file decoy earns only file-match strength
    // (no line precision available from this channel).
    let pyr = run(
        "test_failure",
        vec![cases[2].1.clone()],
        vec![],
        "fresh",
        &cases[2].0,
    );
    let decoy = score_of(&pyr, "py_helper");
    assert_eq!(decoy.evidence[0].kind, EvidenceKind::DiagnosticFileMatch);
    assert!(decoy.score < score_of(&pyr, "add").score);

    // TypeScript: failures arrive through the file:line channel (no
    // test-name marker in today's parsers) — exact location compensates.
    let ts = {
        let mut f = Fx::new("lang_ts");
        f.func("addTs", "src/math.ts", 1, 8);
        f.test("testAdd", None, &["sym::src/math.ts::addTs_function@1"]);
        f.build()
    };
    let r = run(
        "unknown_failure",
        vec![diag(Some("src/math.ts"), Some(5), None)],
        vec![],
        "fresh",
        &ts,
    );
    let h = score_of(&r, "addTs");
    assert!(h
        .evidence
        .iter()
        .any(|e| e.kind == EvidenceKind::ExactDiagnosticLocation));
    assert!((h.score - 0.45).abs() < EPS);
    assert_eq!(h.confidence, "moderate");
}

// ── 8. determinism ───────────────────────────────────────────────────────

/// Identical inputs (rebuilt store included) produce byte-identical
/// serialized analyses.
#[test]
fn serialized_output_is_byte_deterministic() {
    let build = || {
        let mut f = Fx::new("det");
        let core = f.func("core", "src/c.rs", 1, 20);
        let peer = f.func("peer", "src/c.rs", 30, 35);
        let check_fn = f.func("check", "src/c.rs", 40, 44);
        f.calls(&check_fn, &core, Some("verified"));
        f.test("check", None, &[&core, &peer]);
        f.build()
    };
    let mk = |s: &FactStore| {
        run(
            "test_failure",
            vec![
                diag(Some("src/c.rs"), Some(3), Some("check")),
                diag(None, None, Some("check")),
            ],
            vec![edit("src/c.rs", &["check"])],
            "stale",
            s,
        )
    };
    let a = mk(&build());
    let b = mk(&build());
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}
