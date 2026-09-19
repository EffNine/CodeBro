//! Evidence collection for candidates. Bounded, sourced, deduplicated.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use super::candidates::{CandidateSet, DiagnosticInput, RecentEditInput};
use super::types::{Candidate, Evidence, EvidenceKind};

/// Everything evidence assembly needs per candidate, precomputed once.
#[derive(Debug, Default)]
pub struct EvidenceBundle {
    pub items: Vec<Evidence>,
    /// Failing tests connected to this candidate (exercised-by or self).
    pub related_tests: Vec<String>,
}

/// Assemble evidence for one candidate. `failing_tests` is the deduped set
/// of runner-identified failing test names from the diagnostics.
#[allow(clippy::too_many_arguments)]
pub fn collect(
    candidate: &Candidate,
    diagnostics: &[DiagnosticInput],
    failing_tests: &[String],
    set: &CandidateSet,
    recent_edits: &[RecentEditInput],
) -> EvidenceBundle {
    let mut bundle = EvidenceBundle::default();

    // --- diagnostic location / file evidence ------------------------------
    for diag in diagnostics {
        let Some(d_file) = &diag.file else { continue };
        let Some(c_file) = &candidate.file else {
            continue;
        };
        if normalize(d_file) != normalize(c_file) {
            continue;
        }
        // Containment uses the candidate's full span (start..=end) and
        // REQUIRES a diagnostic line: a file-only diagnostic (common in
        // pytest short summaries) carries no location precision, so it can
        // never earn the exact-location weight — only file-match strength.
        let within_span = diag.line.is_some_and(|dl| {
            let start = candidate.line.unwrap_or(0);
            let end = candidate.end_line.unwrap_or(start);
            dl >= start && dl <= end
        });
        if within_span {
            bundle.items.push(Evidence {
                kind: EvidenceKind::ExactDiagnosticLocation,
                source: format!("loc:{}", c_file),
                detail: format!(
                    "diagnostic points at {}:{} (symbol {})",
                    d_file,
                    diag.line.unwrap_or(0),
                    candidate.name
                ),
            });
        } else {
            bundle.items.push(Evidence {
                kind: EvidenceKind::DiagnosticFileMatch,
                source: format!("file:{}", c_file),
                detail: format!("diagnostic file matches {}", c_file),
            });
        }
    }

    // Channel dominance: once a diagnostic lands inside the candidate's
    // span, file-level matches from other diagnostics about the SAME file
    // carry no additional attribution signal for this candidate — they are
    // weaker representations of overlapping observations. Keep only the
    // strongest location channel so repeated representations of one event
    // cannot stack into inflated confidence.
    let has_exact = bundle
        .items
        .iter()
        .any(|e| e.kind == EvidenceKind::ExactDiagnosticLocation);
    if has_exact {
        bundle
            .items
            .retain(|e| e.kind != EvidenceKind::DiagnosticFileMatch);
    }

    // --- failing-test evidence --------------------------------------------
    for runner_name in failing_tests {
        for test_name in set.canonical_failing_names(runner_name) {
            if let Some(tests) = set.exercised_by.get(&candidate.symbol_id) {
                if tests.iter().any(|t| t == &test_name) {
                    bundle.items.push(Evidence {
                        kind: EvidenceKind::FailingTestExercisesSymbol,
                        source: format!("test:{test_name}"),
                        detail: format!(
                            "failing test `{test_name}` exercises `{}`",
                            candidate.name
                        ),
                    });
                    if !bundle.related_tests.contains(&test_name) {
                        bundle.related_tests.push(test_name.clone());
                    }
                }
            }
            if set.test_self.get(&test_name).map(|s| s.as_str())
                == Some(candidate.symbol_id.as_str())
            {
                bundle.items.push(Evidence {
                    kind: EvidenceKind::CandidateIsFailingTest,
                    source: format!("test:{test_name}"),
                    detail: format!(
                        "`{}` is itself the failing test — the test may be wrong",
                        candidate.name
                    ),
                });
                if !bundle.related_tests.contains(&test_name) {
                    bundle.related_tests.push(test_name.clone());
                }
            }
        }
    }

    // --- recent-change evidence -------------------------------------------
    // Canonicalize runner-reported failing names (they may carry module
    // paths) before intersecting with the advisory's recommendations.
    let failing_canonical: std::collections::HashSet<String> = failing_tests
        .iter()
        .flat_map(|t| set.canonical_failing_names(t))
        .collect();
    // Tests that exercise THIS candidate (canonical names).
    let cand_tests: std::collections::HashSet<String> = set
        .exercised_by
        .get(&candidate.symbol_id)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let candidate_recommended = |edit: &RecentEditInput| -> bool {
        // The precise causal hint: this edit recommended failing test T,
        // and T exercises THIS symbol. Generic file-sharing does NOT
        // qualify — otherwise every sibling in the file inflates equally.
        failing_canonical
            .iter()
            .any(|t| edit.recommended_tests.iter().any(|r| r == t) && cand_tests.contains(t))
    };
    for edit in recent_edits {
        let matches_file = candidate
            .file
            .as_deref()
            .map(|f| normalize(f) == normalize(&edit.path))
            .unwrap_or(false);
        if !matches_file {
            continue;
        }
        if candidate_recommended(edit) {
            bundle.items.push(Evidence {
                kind: EvidenceKind::RecentChangeRecommendedFailingTest,
                source: format!("edit:{}", edit.path),
                detail: format!(
                    "{} was edited {}s ago and its advisory recommended the failing test",
                    edit.path, edit.seconds_ago
                ),
            });
        } else {
            bundle.items.push(Evidence {
                kind: EvidenceKind::RecentChangeMatch,
                source: format!("edit:{}", edit.path),
                detail: format!("{} changed recently ({}s ago)", edit.path, edit.seconds_ago),
            });
        }
    }

    // Deterministic ordering; caller dedupes and caps.
    bundle.items.sort_by(|a, b| {
        (a.kind.as_str(), a.source.clone()).cmp(&(b.kind.as_str(), b.source.clone()))
    });
    bundle.related_tests.sort();
    bundle.related_tests.dedup();
    bundle
}

fn normalize(p: &str) -> String {
    p.trim_start_matches("./").to_string()
}
