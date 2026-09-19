//! Root-cause hypothesis engine.
//!
//! Transforms already-verified runtime evidence — parsed diagnostics,
//! failure classification, fact-store linkage, impact relationships, and
//! in-session recent-change correlation — into deterministic, explainable,
//! ranked hypotheses for the host agent.
//!
//! Invariants:
//! - No LLM, no embeddings, no persistence. Hypotheses are derived runtime
//!   evidence and are never promoted into the fact store or memory.
//! - Deterministic: identical inputs produce byte-identical output.
//! - Bounded: every loop is capped (see [`types`] constants).
//! - "Changed recently" is evidence; "caused the failure" is never claimed.

pub mod candidates;
pub mod evidence;
pub mod ranking;
pub mod types;

use crate::fact_store::FactStore;
use crate::sandbox::VerificationResult;
use types::{AnalysisStatus, Candidate, EvidenceKind, Hypothesis, RootCauseAnalysis};

/// Everything the engine needs. The caller (thin MCP handler) supplies the
/// verified verification result, the cached fact store, precomputed
/// freshness, and the session's recent-edit snapshot.
pub struct RootCauseInput<'a> {
    pub verification: &'a VerificationResult,
    pub store: &'a FactStore,
    pub workspace_root: &'a std::path::Path,
    /// `fresh` | `stale` | `unknown` — computed by the caller via the
    /// existing freshness helper; never recomputed here.
    pub freshness: &'a str,
    pub recent_edits: &'a [candidates::RecentEditInput],
}

/// Run deterministic hypothesis generation and ranking.
pub fn analyze_root_cause(input: RootCauseInput<'_>) -> RootCauseAnalysis {
    let classification = input
        .verification
        .classification
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    let diagnostics = candidates::diagnostics_from(&input.verification.diagnostics);
    let failing_tests: Vec<String> = {
        let mut names: Vec<String> = diagnostics.iter().filter_map(|d| d.test.clone()).collect();
        names.sort();
        names.dedup();
        names
    };

    let set = candidates::generate(input.store, &diagnostics, input.recent_edits);

    let mut limitations: Vec<String> = Vec::new();
    if input.freshness != "fresh" {
        // Surfaced per-hypothesis by ranking too; top-level copy for quick reading.
        limitations.push(format!("fact store freshness: {}", input.freshness));
    }

    let mut scored: Vec<Hypothesis> = Vec::new();
    for candidate in &set.candidates {
        let bundle = evidence::collect(
            candidate,
            &diagnostics,
            &failing_tests,
            &set,
            input.recent_edits,
        );
        if bundle.items.is_empty() {
            continue;
        }
        let ambiguous_name = set.ambiguous_names.contains(&candidate.name);
        let base = ranking::base_score(&bundle.items);
        let (score, mut limits) = ranking::adjusted(base, ambiguous_name, input.freshness);
        let confidence = ranking::confidence_tier(score);
        limitations.append(&mut limits);
        scored.push(Hypothesis {
            candidate: Candidate {
                symbol_id: candidate.symbol_id.clone(),
                name: candidate.name.clone(),
                file: candidate.file.clone(),
                line: candidate.line,
                end_line: candidate.end_line,
            },
            score,
            confidence: confidence.to_string(),
            evidence: ranking::finalize_evidence(bundle.items),
            impact_context: Vec::new(),
            related_recent_changes: recent_changes_for(candidate, input.recent_edits),
            related_tests: bundle.related_tests,
            ambiguous_name,
        });
    }
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.candidate.symbol_id.cmp(&b.candidate.symbol_id))
    });
    scored.truncate(types::MAX_HYPOTHESES);

    // Bounded impact enrichment for the top candidates only.
    for h in scored.iter_mut().take(types::MAX_IMPACT_ENRICHED) {
        let extra = enrich_with_impact(h, input.store);
        for e in extra {
            if !h
                .evidence
                .iter()
                .any(|x| x.kind == e.kind && x.source == e.source)
            {
                h.evidence.push(e);
            }
        }
        // Re-rank after enrichment so relationship evidence counts.
        // Truncate honestly first: the score must be computed over exactly
        // the evidence list that is serialized, and the per-hypothesis cap
        // must hold after enrichment items are added.
        h.evidence = ranking::finalize_evidence(std::mem::take(&mut h.evidence));
        let base = ranking::base_score(&h.evidence);
        let (score, _) = ranking::adjusted(base, h.ambiguous_name, input.freshness);
        h.score = score;
        h.confidence = ranking::confidence_tier(score).to_string();
    }
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.candidate.symbol_id.cmp(&b.candidate.symbol_id))
    });

    // Evidence summary across retained hypotheses.
    let mut counts: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for h in &scored {
        for e in &h.evidence {
            *counts.entry(e.kind.as_str()).or_default() += 1;
        }
    }
    let evidence_summary: Vec<(String, usize)> = counts
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

    if scored.is_empty() {
        limitations.push(
            "no usable evidence: no diagnostics with locations, no failing-test linkage, \
             no related recent edits"
                .to_string(),
        );
    } else if !scored.iter().any(|h| h.confidence == "strong") {
        limitations.push(
            "evidence present but below strong threshold — treat as leads, not answers".to_string(),
        );
    }
    // Deduplicate per-hypothesis limitations into one clean top-level set.
    limitations.sort();
    limitations.dedup();
    limitations.truncate(8);
    if !set.ambiguous_names.is_empty() && !scored.is_empty() {
        limitations.push(format!("ambiguous symbol names present: {}", {
            let mut v: Vec<String> = set
                .ambiguous_names
                .iter()
                .filter(|n| scored.iter().any(|h| &h.candidate.name == *n))
                .cloned()
                .collect();
            v.sort();
            v.join(", ")
        }));
    }

    let status = if scored.is_empty() {
        AnalysisStatus::InsufficientEvidence
    } else if scored.iter().any(|h| h.confidence == "strong") {
        AnalysisStatus::Hypotheses
    } else {
        AnalysisStatus::WeakSignals
    };

    RootCauseAnalysis {
        status,
        failure_classification: classification,
        hypotheses: scored,
        evidence_summary,
        freshness: input.freshness.to_string(),
        limitations,
    }
}

fn recent_changes_for(
    candidate: &Candidate,
    edits: &[candidates::RecentEditInput],
) -> Vec<serde_json::Value> {
    let Some(cf) = &candidate.file else {
        return Vec::new();
    };
    edits
        .iter()
        .filter(|e| e.path.trim_start_matches("./") == cf.trim_start_matches("./"))
        .map(|e| {
            serde_json::json!({
                "path": e.path,
                "seconds_ago": e.seconds_ago,
            })
        })
        .collect()
}

/// Bounded impact-engine reuse: depth-1 edges around the candidate,
/// converted to compact summaries plus relationship evidence when an edge
/// connects to failing evidence (test symbol or diagnostic symbol).
fn enrich_with_impact(h: &mut Hypothesis, store: &FactStore) -> Vec<types::Evidence> {
    use crate::impact::{analyze, ImpactOptions, ImpactTarget};

    let target_id = match crate::impact::resolve_symbol_name(store, &h.candidate.name) {
        Ok(crate::impact::ImpactTarget::Symbol(id)) => id,
        _ => crate::engineering_facts::SymbolId::new(h.candidate.symbol_id.clone()),
    };

    let opts = ImpactOptions {
        depth: types::IMPACT_DEPTH,
        direction: "both".to_string(),
        max_results: types::IMPACT_MAX_RESULTS,
        include_references: false,
        ..Default::default()
    };
    let result = analyze(store, ImpactTarget::Symbol(target_id), &opts, None);

    let mut extras = Vec::new();
    let mut summaries = Vec::new();
    for e in result
        .direct_relationships
        .iter()
        .take(types::IMPACT_MAX_RESULTS)
    {
        summaries.push(types::ImpactEdgeSummary {
            kind: e.relationship_kind.clone(),
            direction: e.direction.clone(),
            other_symbol: e.target_name.clone(),
            provenance: format!("{:?}", e.provenance).to_lowercase(),
            depth: e.depth,
        });
        // Relationship evidence only when it connects to failing evidence:
        // the other endpoint is one of this hypothesis's related tests.
        if h.related_tests.iter().any(|t| t == &e.target_name) {
            let prov_str = format!("{:?}", e.provenance).to_lowercase();
            let kind = if prov_str == "verified" {
                EvidenceKind::VerifiedRelationship
            } else {
                EvidenceKind::HeuristicRelationship
            };
            extras.push(types::Evidence {
                kind,
                source: format!("rel:{}/{}", h.candidate.name, e.target_name),
                detail: format!(
                    "{} {} {} (depth {}, provenance {}, direct)",
                    h.candidate.name, e.direction, e.target_name, e.depth, prov_str
                ),
            });
        }
    }
    h.impact_context = summaries;
    extras
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugging::candidates::{DiagnosticInput, RecentEditInput};
    use crate::engineering_facts::{
        FactId, FactsBuilder, ModuleFact, ModuleId, SourceLocation, SymbolFact, SymbolId,
        SymbolKind, TestFact, TestId, WorkspaceFact, WorkspaceId,
    };
    use crate::sandbox::{ParsedDiagnostic, VerificationResult};

    fn store() -> FactStore {
        let ws = WorkspaceId::new("ws::t");
        let mut b = FactsBuilder::new();
        b.add_workspace(WorkspaceFact::new(ws, "t"));
        let mut m = ModuleFact::new(ModuleId::new("mod::src/lib.rs"), "src::lib");
        m.path = Some("src/lib.rs".to_string());
        b.add_module(m);
        // add() at lines 1..3 — the production symbol.
        let add_id = "sym::src/lib.rs::add_function@1";
        let mut sf = SymbolFact::new(
            SymbolId::new(add_id.to_string()),
            "add",
            SymbolKind::Function,
        );
        sf.location = SourceLocation::new().with_file("src/lib.rs").with_span(
            crate::engineering_facts::Span::new(
                crate::engineering_facts::Position::new(1, 0),
                crate::engineering_facts::Position::new(3, 0),
            ),
        );
        sf.location.line = Some(1);
        b.add_symbol(sf);
        // helper() elsewhere in the same file (decoy for file-match).
        let h_id = "sym::src/lib.rs::helper_function@9";
        let mut hf = SymbolFact::new(
            SymbolId::new(h_id.to_string()),
            "helper",
            SymbolKind::Function,
        );
        hf.location = SourceLocation::new()
            .with_file("src/lib.rs")
            .with_point(9, 0);
        hf.location.span = Some(crate::engineering_facts::Span::new(
            crate::engineering_facts::Position::new(9, 0),
            crate::engineering_facts::Position::new(10, 0),
        ));
        b.add_symbol(hf);
        // failing test exercising add()
        let mut tf = TestFact::new(TestId::new("test::f::adds"), "adds");
        tf.tested.push(SymbolId::new(add_id.to_string()));
        tf.location = Some(
            SourceLocation::new()
                .with_file("src/lib.rs")
                .with_point(5, 0),
        );
        b.add_test(tf);
        FactStore::build(b.build())
    }

    fn diag(file: Option<&str>, line: Option<u32>, test: Option<&str>) -> ParsedDiagnostic {
        ParsedDiagnostic {
            severity: "failure".to_string(),
            code: None,
            message: String::new(),
            file: file.map(|s| s.to_string()),
            line,
            column: None,
            test: test.map(|s| s.to_string()),
        }
    }

    fn verification(classification: &str, diags: Vec<ParsedDiagnostic>) -> VerificationResult {
        use rmcp::model::{CallToolResult, ContentBlock};
        // Build via the public constructor then override classification.
        let execution = crate::sandbox::ExecutionResult::from_local(
            "cargo test",
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

    fn run(
        classification: &str,
        diags: Vec<ParsedDiagnostic>,
        edits: Vec<RecentEditInput>,
        freshness: &str,
    ) -> RootCauseAnalysis {
        run_with_store(classification, diags, edits, freshness, store())
    }

    fn run_with_store(
        classification: &str,
        diags: Vec<ParsedDiagnostic>,
        edits: Vec<RecentEditInput>,
        freshness: &str,
        s: FactStore,
    ) -> RootCauseAnalysis {
        let v = verification(classification, diags);
        analyze_root_cause(RootCauseInput {
            verification: &v,
            store: &s,
            workspace_root: std::path::Path::new("/tmp/nowhere"),
            freshness,
            recent_edits: &edits,
        })
    }

    fn edit(path: &str, recs: &[&str]) -> RecentEditInput {
        RecentEditInput {
            path: path.to_string(),
            seconds_ago: 3,
            recommended_tests: recs.iter().map(|s| s.to_string()).collect(),
        }
    }

    // 1. Exact diagnostic location outranks generic match.
    #[test]
    fn exact_location_outranks_generic() {
        let r = run(
            "compile_error",
            vec![diag(Some("src/lib.rs"), Some(2), None)],
            vec![],
            "fresh",
        );
        assert!(!r.hypotheses.is_empty());
        assert_eq!(r.hypotheses[0].candidate.name, "add");
        assert!(r.hypotheses[0]
            .evidence
            .iter()
            .any(|e| e.kind == crate::debugging::types::EvidenceKind::ExactDiagnosticLocation));
    }

    // 2. Failing test exercising a symbol boosts it.
    #[test]
    fn failing_test_exercise_boosts_candidate() {
        let r = run(
            "test_failure",
            vec![diag(None, None, Some("adds"))],
            vec![],
            "fresh",
        );
        let top = &r.hypotheses[0];
        assert_eq!(top.candidate.name, "add", "{:?}", top.candidate);
        assert!(top
            .evidence
            .iter()
            .any(|e| e.kind == crate::debugging::types::EvidenceKind::FailingTestExercisesSymbol));
        assert_eq!(top.related_tests, vec!["adds"]);
    }

    // 3. Recency increases ranking but never dominates precise evidence.
    #[test]
    fn recency_supports_but_does_not_dominate() {
        // Candidate with ONLY recency.
        let recent_only = run(
            "unknown_failure",
            vec![diag(None, None, None)],
            vec![edit("src/lib.rs", &[])],
            "fresh",
        );
        let recent_score = recent_only
            .hypotheses
            .iter()
            .find(|h| h.candidate.name == "add")
            .map(|h| h.score)
            .unwrap_or(0.0);

        // Candidate with exact diagnostic location (no recency).
        let exact = run(
            "compile_error",
            vec![diag(Some("src/lib.rs"), Some(2), None)],
            vec![],
            "fresh",
        );
        let exact_score = exact
            .hypotheses
            .iter()
            .find(|h| h.candidate.name == "add")
            .map(|h| h.score)
            .unwrap();
        assert!(recent_score > 0.0, "recency is a signal");
        assert!(exact_score > recent_score, "precise beats recency");
    }

    // 4/5. Relationship weights + direct-over-transitive live in the weight
    // table; asserted directly in ranking::tests.

    // 6. Duplicate evidence does not inflate (covered in ranking::tests too).
    #[test]
    fn duplicate_sources_deduplicate_in_pipeline() {
        let r = run(
            "test_failure",
            vec![
                diag(None, None, Some("adds")),
                diag(None, None, Some("adds")),
            ],
            vec![],
            "fresh",
        );
        let top = &r.hypotheses[0];
        let count = top
            .evidence
            .iter()
            .filter(|e| {
                e.kind == crate::debugging::types::EvidenceKind::FailingTestExercisesSymbol
                    && e.source == "test:adds"
            })
            .count();
        assert_eq!(count, 1);
    }

    // 7. Ambiguity reduces confidence and surfaces a limitation.
    #[test]
    fn ambiguous_names_lower_confidence_and_note_limitation() {
        // Two failing-test hints that map to different symbols with the same
        // name is hard to build honestly here; instead verify the penalty
        // path via ranking::adjusted (covered there) and that the analysis
        // reports ambiguity limitations when present in the candidate set.
        let r = run(
            "test_failure",
            vec![diag(None, None, Some("adds"))],
            vec![],
            "fresh",
        );
        // No ambiguity in this fixture — limitation must NOT appear.
        assert!(r.limitations.iter().all(|l| !l.contains("ambiguous")));
        // Penalty math is pinned by ranking unit tests.
        let (score, limits) = crate::debugging::ranking::adjusted(0.60, true, "fresh");
        assert!(score < 0.60 && limits.iter().any(|l| l.contains("ambiguous")));
    }

    // 8. Stale facts reduce confidence / produce limitation.
    #[test]
    fn stale_facts_scale_down_and_surface_limitation() {
        let fresh = run(
            "test_failure",
            vec![diag(None, None, Some("adds"))],
            vec![],
            "fresh",
        );
        let stale = run(
            "test_failure",
            vec![diag(None, None, Some("adds"))],
            vec![],
            "stale",
        );
        let f = fresh.hypotheses[0].score;
        let s = stale.hypotheses[0].score;
        assert!(s < f);
        assert!(stale.freshness == "stale");
        assert!(stale.limitations.iter().any(|l| l.contains("stale")));
    }

    // 9. No evidence → no strong hypothesis.
    #[test]
    fn no_evidence_yields_insufficient_state() {
        let r = run("success", vec![], vec![], "fresh");
        assert_eq!(
            r.status,
            crate::debugging::types::AnalysisStatus::InsufficientEvidence
        );
        assert!(r.hypotheses.is_empty());
        assert!(r
            .limitations
            .iter()
            .any(|l| l.contains("no usable evidence")));
    }

    // 10. Determinism across runs.
    #[test]
    fn deterministic_output() {
        let a = run(
            "test_failure",
            vec![
                diag(None, None, Some("adds")),
                diag(Some("src/lib.rs"), Some(2), Some("adds")),
            ],
            vec![edit("src/lib.rs", &["adds"])],
            "fresh",
        );
        let b = run(
            "test_failure",
            vec![
                diag(None, None, Some("adds")),
                diag(Some("src/lib.rs"), Some(2), Some("adds")),
            ],
            vec![edit("src/lib.rs", &["adds"])],
            "fresh",
        );
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    // 11. max hypotheses respected.
    #[test]
    fn hypothesis_cap_respected() {
        // Many distinct edited files → many candidates; cap must hold.
        let edits: Vec<RecentEditInput> = (0..12)
            .map(|i| edit(&format!("src/file{i}.rs"), &[]))
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let ws = WorkspaceId::new("ws::many");
        let mut b = FactsBuilder::new();
        b.add_workspace(WorkspaceFact::new(ws, "many"));
        for i in 0..12 {
            let p = format!("src/file{i}.rs");
            let mut m = ModuleFact::new(ModuleId::new(format!("mod::{p}")), p.clone());
            m.path = Some(p.clone());
            b.add_module(m);
            let id = format!("sym::{p}::f{i}_function@1");
            let mut sf = SymbolFact::new(
                SymbolId::new(id.to_string()),
                format!("f{i}"),
                SymbolKind::Function,
            );
            sf.location = SourceLocation::new().with_file(p).with_point(1, 0);
            b.add_symbol(sf);
        }
        let big_store = FactStore::build(b.build());
        let _ = dir;

        let v = verification("unknown_failure", vec![diag(None, None, None)]);
        let r = analyze_root_cause(RootCauseInput {
            verification: &v,
            store: &big_store,
            workspace_root: std::path::Path::new("/tmp/nowhere"),
            freshness: "fresh",
            recent_edits: &edits,
        });
        assert!(r.hypotheses.len() <= crate::debugging::types::MAX_HYPOTHESES);
    }

    // 12. Malformed/empty input handled safely.
    #[test]
    fn empty_and_malformed_inputs_are_safe() {
        let r = run("success", vec![], vec![], "fresh");
        assert_eq!(
            r.status,
            crate::debugging::types::AnalysisStatus::InsufficientEvidence
        );
        let weird = ParsedDiagnostic {
            severity: String::new(),
            code: None,
            message: String::new(),
            file: Some(String::new()),
            line: None,
            column: None,
            test: Some(String::new()),
        };
        let r2 = run("unknown_failure", vec![weird], vec![edit("", &[])], "bogus");
        // Must not panic; empty-file diagnostics match nothing meaningful.
        assert!(matches!(
            r2.status,
            crate::debugging::types::AnalysisStatus::InsufficientEvidence
                | crate::debugging::types::AnalysisStatus::WeakSignals
        ));
    }

    // 13. Cross-language candidates follow existing evidence quality.
    #[test]
    fn cross_language_candidates_flow() {
        // Python-style paths through the same pipeline.
        let py_store_store = || {
            let ws = WorkspaceId::new("ws::py");
            let mut b = FactsBuilder::new();
            b.add_workspace(WorkspaceFact::new(ws, "py"));
            let mut m = ModuleFact::new(ModuleId::new("mod::helpers.py"), "helpers");
            m.path = Some("helpers.py".to_string());
            b.add_module(m);
            let id = "sym::helpers.py::add_function@1";
            let mut sf =
                SymbolFact::new(SymbolId::new(id.to_string()), "add", SymbolKind::Function);
            sf.location = SourceLocation::new()
                .with_file("helpers.py")
                .with_point(1, 0);
            b.add_symbol(sf);
            let mut tf = TestFact::new(TestId::new("test::t::test_add"), "test_add");
            tf.tested.push(SymbolId::new(id.to_string()));
            b.add_test(tf);
            FactStore::build(b.build())
        };
        let v = verification("test_failure", vec![diag(None, None, Some("test_add"))]);
        let r = analyze_root_cause(RootCauseInput {
            verification: &v,
            store: &py_store_store(),
            workspace_root: std::path::Path::new("/tmp/nowhere"),
            freshness: "fresh",
            recent_edits: &[],
        });
        assert_eq!(r.hypotheses[0].candidate.name, "add");
        assert_eq!(
            r.hypotheses[0].candidate.file.as_deref(),
            Some("helpers.py")
        );
    }

    // 14. Memory isolation is enforced structurally — see
    // crates/mcp-server/tests/debugging_isolation.rs (source scan) plus the
    // module's total absence of engineering_memory imports.

    // 15. Impact reuse: enrichment attaches impact_context from real engine.
    #[tokio::test]
    async fn impact_enrichment_uses_engine_results() {
        // Build a verified call edge adds→add so enrichment can attach
        // relationship evidence when the related test matches.
        let s = store();
        let mut b = FactsBuilder::new();
        let _ = &s;
        // (Rebuild minimal store with relationship.)
        b.add_workspace(WorkspaceFact::new(WorkspaceId::new("ws::r"), "r"));
        let mut m = ModuleFact::new(ModuleId::new("mod::src/lib.rs"), "src::lib");
        m.path = Some("src/lib.rs".to_string());
        b.add_module(m);
        let add_id = "sym::src/lib.rs::add_function@1";
        let mut sf = SymbolFact::new(
            SymbolId::new(add_id.to_string()),
            "add",
            SymbolKind::Function,
        );
        sf.location = SourceLocation::new()
            .with_file("src/lib.rs")
            .with_point(1, 0);
        b.add_symbol(sf);
        let adds_id = "sym::src/lib.rs::adds_function@5";
        let mut ts = SymbolFact::new(
            SymbolId::new(adds_id.to_string()),
            "adds",
            SymbolKind::Function,
        );
        ts.location = SourceLocation::new()
            .with_file("src/lib.rs")
            .with_point(5, 0);
        b.add_symbol(ts);
        let mut rel = crate::engineering_facts::RelationshipFact::new(
            crate::engineering_facts::RelationshipId::new(format!("rel::{}/{}", adds_id, add_id)),
            crate::engineering_facts::RelationshipKind::Calls,
            FactId::Symbol(SymbolId::new(adds_id.to_string())),
            FactId::Symbol(SymbolId::new(add_id.to_string())),
        );
        rel.metadata = crate::engineering_facts::metadata::FactMetadata::builder()
            .attr("provenance", "verified")
            .build();
        b.add_relationship(rel);
        let mut tf = TestFact::new(TestId::new("test::f::adds"), "adds".to_string());
        tf.tested.push(SymbolId::new(add_id.to_string()));
        tf.target = Some(FactId::Symbol(SymbolId::new(adds_id.to_string())));
        tf.location = Some(
            SourceLocation::new()
                .with_file("src/lib.rs")
                .with_point(5, 0),
        );
        b.add_test(tf);
        let rich = FactStore::build(b.build());

        let v = verification("test_failure", vec![diag(None, None, Some("adds"))]);
        let r = analyze_root_cause(RootCauseInput {
            verification: &v,
            store: &rich,
            workspace_root: std::path::Path::new("/tmp/nowhere"),
            freshness: "fresh",
            recent_edits: &[],
        });
        let top = r
            .hypotheses
            .iter()
            .find(|h| h.candidate.name == "add")
            .expect("add hypothesized");
        assert!(
            !top.impact_context.is_empty(),
            "impact context must be attached via the real engine"
        );
        assert!(top
            .evidence
            .iter()
            .any(|e| e.kind == crate::debugging::types::EvidenceKind::VerifiedRelationship));
    }
}

#[cfg(test)]
mod live_shape_probe {
    use super::tests::*;
}

#[cfg(test)]
mod live_pipeline_regression {
    use super::*;
    use crate::engineering_facts::{
        FactId, FactsBuilder, ModuleFact, ModuleId, SourceLocation, SymbolFact, SymbolId,
        SymbolKind, TestFact, TestId, WorkspaceFact, WorkspaceId,
    };
    use crate::sandbox::{ParsedDiagnostic, VerificationResult};

    /// End-to-end through the REAL init pipeline (macro-wrapped calls,
    /// module-path test hints) with diagnostics shaped exactly like live
    /// libtest output.
    #[tokio::test]
    async fn rca_matches_live_failure_shape() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"live\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"),
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn adds() {\n        assert_eq!(add(2, 3), 6);\n    }\n}\n").unwrap();
        crate::init::run(dir.path()).expect("init ok");

        let v = {
            let execution = crate::sandbox::ExecutionResult::from_local(
                "cargo test adds",
                "/tmp/live",
                "",
                "",
                101,
                250,
                false,
                false,
                std::collections::HashMap::new(),
            );
            let mut vr = VerificationResult::from_execution(execution);
            vr.verified = false;
            vr.classification = Some("test_failure".to_string());
            vr.diagnostics = vec![
                ParsedDiagnostic {
                    severity: "failure".into(),
                    code: None,
                    message: "test failed: tests::adds".into(),
                    file: Some("src/lib.rs".into()),
                    line: Some(10),
                    column: Some(9),
                    test: Some("tests::adds".into()),
                },
                ParsedDiagnostic {
                    severity: "error".into(),
                    code: Some("panic".into()),
                    message: "panic in tests::adds".into(),
                    file: Some("src/lib.rs".into()),
                    line: Some(10),
                    column: Some(9),
                    test: None,
                },
            ];
            vr
        };
        // Build the store exactly as the server does (public path).
        let model_bytes = std::fs::read(dir.path().join(".codebro/facts.json")).unwrap();
        let model: crate::engineering_facts::FactsModel =
            serde_json::from_slice(&model_bytes).unwrap();
        let s = FactStore::from_model(&model);
        let r = analyze_root_cause(RootCauseInput {
            verification: &v,
            store: &s,
            workspace_root: dir.path(),
            freshness: "stale",
            recent_edits: &[],
        });
        let names: Vec<&str> = r
            .hypotheses
            .iter()
            .map(|h| h.candidate.name.as_str())
            .collect();
        assert!(
            names.contains(&"add"),
            "add must be hypothesized, got {names:?}"
        );
        assert!(names.contains(&"adds"), "{names:?}");
        assert!(names.contains(&"add"), "{names:?}");
        let adds_h = r
            .hypotheses
            .iter()
            .find(|h| h.candidate.name == "adds")
            .unwrap();
        assert!(adds_h
            .evidence
            .iter()
            .any(|e| e.kind == crate::debugging::types::EvidenceKind::CandidateIsFailingTest));
        let add_h = r
            .hypotheses
            .iter()
            .find(|h| h.candidate.name == "add")
            .unwrap();
        assert!(add_h
            .evidence
            .iter()
            .any(|e| e.kind == crate::debugging::types::EvidenceKind::FailingTestExercisesSymbol));
        assert!(
            !add_h.impact_context.is_empty(),
            "impact enrichment attached"
        );
    }
}
