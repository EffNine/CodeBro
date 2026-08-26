//! Deterministic, transparent scoring.
//!
//! score = Σ unique (kind, source) weights, clamped to [0, 0.98],
//!         − 0.15 ambiguity penalty,
//!         × 0.85 when facts are stale, × 0.95 when freshness unknown.
//!
//! No ML, no LLM, no hidden inputs. Duplicate evidence from the same
//! underlying signal cannot inflate a score because dedup keys on
//! (kind, source).

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use super::types::{Evidence, EvidenceKind};

/// Penalty applied once per hypothesis whose candidate name is ambiguous.
pub const AMBIGUITY_PENALTY: f64 = 0.15;
pub const STALE_MULTIPLIER: f64 = 0.85;
pub const UNKNOWN_FRESHNESS_MULTIPLIER: f64 = 0.95;
pub const SCORE_CAP: f64 = 0.98;

/// Sum deduplicated evidence weights for one hypothesis.
pub fn base_score(evidence: &[Evidence]) -> f64 {
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut total = 0.0;
    for e in evidence {
        let key = (e.kind.as_str().to_string(), e.source.clone());
        if seen.insert(key) {
            total += e.kind.weight();
        }
    }
    total.min(SCORE_CAP)
}

/// Apply ambiguity and freshness adjustments.
pub fn adjusted(base: f64, ambiguous_name: bool, freshness: &str) -> (f64, Vec<String>) {
    let mut limitations = Vec::new();
    let mut score = base;
    if ambiguous_name {
        score -= AMBIGUITY_PENALTY;
        limitations.push("candidate name is ambiguous across symbols".to_string());
    }
    match freshness {
        "stale" => {
            score *= STALE_MULTIPLIER;
            limitations.push("fact store is stale relative to the working tree".to_string());
        }
        "unknown" => {
            score *= UNKNOWN_FRESHNESS_MULTIPLIER;
            limitations.push("fact-store freshness could not be determined".to_string());
        }
        _ => {}
    }
    let score = score.clamp(0.0, SCORE_CAP);
    (score, limitations)
}

/// Confidence tier from an adjusted score.
pub fn confidence_tier(score: f64) -> &'static str {
    if score >= 0.55 {
        "strong"
    } else if score >= 0.30 {
        "moderate"
    } else if score > f64::EPSILON {
        "weak"
    } else {
        "insufficient"
    }
}

/// Deduplicate, order deterministically, and cap the evidence list.
pub fn finalize_evidence(mut items: Vec<Evidence>) -> Vec<Evidence> {
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    items.sort_by(|a, b| {
        (b.kind.weight())
            .partial_cmp(&a.kind.weight())
            .unwrap()
            .then_with(|| a.kind.as_str().cmp(b.kind.as_str()))
            .then_with(|| a.source.cmp(&b.source))
    });
    items.retain(|e| seen.insert((e.kind.as_str().to_string(), e.source.clone())));
    items.truncate(super::types::MAX_EVIDENCE_PER_HYPOTHESIS);
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: EvidenceKind, source: &str) -> Evidence {
        Evidence {
            kind,
            source: source.to_string(),
            detail: String::new(),
        }
    }

    #[test]
    fn duplicates_do_not_inflate() {
        let items = vec![
            ev(EvidenceKind::FailingTestExercisesSymbol, "test:t1"),
            ev(EvidenceKind::FailingTestExercisesSymbol, "test:t1"),
        ];
        assert!((base_score(&items) - 0.30).abs() < 1e-9);
    }

    #[test]
    fn verified_outweighs_heuristic_relationships() {
        assert!(
            EvidenceKind::VerifiedRelationship.weight()
                > EvidenceKind::HeuristicRelationship.weight()
        );
    }

    #[test]
    fn exact_location_outranks_generic_file_match() {
        let exact = base_score(&[ev(EvidenceKind::ExactDiagnosticLocation, "diag:0")]);
        let generic = base_score(&[ev(EvidenceKind::DiagnosticFileMatch, "diag:0")]);
        assert!(exact > generic);
    }

    #[test]
    fn recency_is_supporting_not_dominant() {
        // Exact location beats any single recency signal…
        let exact = base_score(&[ev(EvidenceKind::ExactDiagnosticLocation, "d")]);
        let recent = base_score(&[ev(EvidenceKind::RecentChangeRecommendedFailingTest, "e")]);
        assert!(exact > recent);
        // …and even combined recency channels stay below exact+test linkage.
        let both_recent = base_score(&[
            ev(EvidenceKind::RecentChangeRecommendedFailingTest, "edit:a"),
            ev(EvidenceKind::RecentChangeMatch, "edit:a"),
        ]);
        let strong = base_score(&[
            ev(EvidenceKind::ExactDiagnosticLocation, "d"),
            ev(EvidenceKind::FailingTestExercisesSymbol, "t"),
        ]);
        assert!(strong > both_recent);
        // Same kind from a DIFFERENT source is independent and counts again.
        let two_tests = base_score(&[
            ev(EvidenceKind::FailingTestExercisesSymbol, "test:t1"),
            ev(EvidenceKind::FailingTestExercisesSymbol, "test:t2"),
        ]);
        assert!((two_tests - 0.60).abs() < 1e-9);
    }

    #[test]
    fn staleness_reduces_score_and_surfaces_limitation() {
        let (score, limits) = adjusted(0.60, false, "stale");
        assert!((score - 0.51).abs() < 1e-6);
        assert!(limits.iter().any(|l| l.contains("stale")));
    }

    #[test]
    fn scores_are_bounded() {
        let many: Vec<Evidence> = (0..50)
            .map(|i| ev(EvidenceKind::ExactDiagnosticLocation, &format!("d{i}")))
            .collect();
        assert!(base_score(&many) <= SCORE_CAP + 1e-9);
        let (score, _) = adjusted(f64::MAX, false, "fresh");
        assert!(score <= SCORE_CAP);
    }
}
